//! Pure deterministic canon event reducers, branch-lineage visibility,
//! coverage-interval masking, watermark time-travel, rollback, and 3-way merge.

use super::comparison::{key_text, normalized_assertion};
use super::events::*;
use super::*;
use crate::models::StoryClock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedScalarV1<T> {
    pub value: Option<T>,
    pub effective_position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

impl<T> Default for ProjectedScalarV1<T> {
    fn default() -> Self {
        Self {
            value: None,
            effective_position: None,
            within_scene_order: None,
            source_event_ids: Vec::new(),
            reducer_version: CANON_REDUCER_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedAttributeV1 {
    pub key: String,
    pub nature: AttributeNature,
    pub scalar_value: Option<ClaimScalar>,
    pub members: BTreeMap<String, ClaimScalar>,
    pub effective_position: Option<CanonPositionV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedInjuryV1 {
    pub injury_id: String,
    pub body_site: String,
    pub severity: InjurySeverity,
    pub status: InjuryStatus,
    pub acquired_at: Option<CanonPositionV1>,
    pub healed_at: Option<CanonPositionV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedPossessionV1 {
    pub item_id: String,
    pub custody: CustodyKind,
    pub held: bool,
    pub counterparty_id: Option<String>,
    pub updated_at: Option<CanonPositionV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedActionOccurrenceV1 {
    pub event_id: String,
    pub verb: String,
    pub mode: ActionMode,
    pub position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub in_world_time: Option<StoryClock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedEntityStateV1 {
    pub subject_id: String,
    /// `None` inside `life_status.value` means unknown, never assumed alive.
    pub life_status: ProjectedScalarV1<LifeStatus>,
    /// `None` inside `location_id.value` means unknown.
    pub location_id: ProjectedScalarV1<String>,
    pub attributes: BTreeMap<String, ProjectedAttributeV1>,
    pub injuries: BTreeMap<String, ProjectedInjuryV1>,
    pub possessions: BTreeMap<String, ProjectedPossessionV1>,
    pub actions: Vec<ProjectedActionOccurrenceV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

impl ProjectedEntityStateV1 {
    pub fn new(subject_id: impl Into<String>) -> Self {
        Self {
            subject_id: subject_id.into(),
            life_status: ProjectedScalarV1::default(),
            location_id: ProjectedScalarV1::default(),
            attributes: BTreeMap::new(),
            injuries: BTreeMap::new(),
            possessions: BTreeMap::new(),
            actions: Vec::new(),
            source_event_ids: Vec::new(),
            reducer_version: CANON_REDUCER_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedRelationshipV1 {
    pub subject_id: String,
    pub other_id: String,
    /// Opening balance observation (`None` when never observed; never invented as zero).
    pub opening_trust: Option<i32>,
    pub opening_tension: Option<i32>,
    pub accumulated_trust_delta: i32,
    pub accumulated_tension_delta: i32,
    /// Resolved total (`Some` iff an opening observation exists).
    pub resolved_trust: Option<i32>,
    pub resolved_tension: Option<i32>,
    /// Per-scene contribution ledger preserved for idempotent scene replacement and branch merge.
    pub scene_delta_ledger: BTreeMap<String, (i32, i32)>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedQuantityV1 {
    pub subject_id: String,
    pub measure: String,
    pub unit: String,
    /// Opening balance (`None` when unknown; never invented as `"0"`).
    pub opening_balance: Option<String>,
    /// Resolved balance (`Some` iff an opening balance was established).
    pub balance: Option<String>,
    pub applied_transactions: BTreeSet<String>,
    pub unapplied_deltas_without_opening_balance: usize,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedKnowledgeUseV1 {
    pub event_id: String,
    pub position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub status_at_use: KnowledgeStatus,
    pub ordering_uncertain: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedDisclosureV1 {
    pub event_id: String,
    pub speaker_id: String,
    pub audience_ids: Vec<String>,
    pub position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub in_world_time: Option<StoryClock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedKnowerEdgeV1 {
    pub knower_id: String,
    pub status: KnowledgeStatus,
    pub learned_at: Option<CanonPositionV1>,
    pub learned_within_scene_order: Option<u32>,
    pub learned_in_world_time: Option<StoryClock>,
    pub source_entity_id: Option<String>,
    pub ordering_uncertain: bool,
    pub uses: Vec<ProjectedKnowledgeUseV1>,
    pub source_event_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedKnowledgeV1 {
    pub proposition_id: String,
    pub knowers: BTreeMap<String, ProjectedKnowerEdgeV1>,
    /// Derived circle of trust: characters with `status == Known` at the cursor.
    pub circle_of_trust: BTreeSet<String>,
    pub disclosures: Vec<ProjectedDisclosureV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedRuleOccurrenceV1 {
    pub event_id: String,
    pub actor_id: String,
    pub operation: RuleOperation,
    pub anchor_id: Option<String>,
    pub position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedRuleConstraintV1 {
    pub rule_id: String,
    pub defined: bool,
    pub preconditions: Vec<String>,
    pub effects: Vec<String>,
    pub anchors: BTreeSet<String>,
    pub occurrences: Vec<ProjectedRuleOccurrenceV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedSceneClockV1 {
    pub position: CanonPositionV1,
    pub scene_id: Option<String>,
    pub clock: StoryClock,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedTimelineV1 {
    pub scene_clocks: BTreeMap<String, ProjectedSceneClockV1>,
    pub latest_clock: Option<ProjectedSceneClockV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedPromiseTransitionV1 {
    pub event_id: String,
    pub subject_id: String,
    pub operation: PromiseOperation,
    pub position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectedThreadV1 {
    pub thread_id: String,
    pub status: PromiseThreadStatus,
    pub planted_at: Option<CanonPositionV1>,
    pub transitions: Vec<ProjectedPromiseTransitionV1>,
    pub source_event_ids: Vec<String>,
    pub reducer_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrderingUncertaintyV1 {
    pub position: CanonPositionV1,
    pub stream_description: String,
    pub event_ids: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReducerAnomalyV1 {
    pub event_id: String,
    pub code: CanonAppendErrorCode,
    pub detail: String,
}

/// Complete rebuildable projection state folded through a cursor and watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonStateViewV1 {
    pub branch_id: String,
    pub lineage: Vec<String>,
    pub cursor: CanonCursorV1,
    pub tier: CanonTier,
    pub decision_watermark: i64,
    pub event_watermark: i64,
    pub reducer_version: u32,
    pub entities: BTreeMap<String, ProjectedEntityStateV1>,
    /// Keyed by `"{subject_id}->{other_id}"`.
    pub relationships: BTreeMap<String, ProjectedRelationshipV1>,
    /// Keyed by `"{subject_id}:{measure}"`.
    pub quantities: BTreeMap<String, ProjectedQuantityV1>,
    /// Keyed by `proposition_id`.
    pub knowledge: BTreeMap<String, ProjectedKnowledgeV1>,
    /// Keyed by `rule_id`.
    pub rules: BTreeMap<String, ProjectedRuleConstraintV1>,
    pub timeline: ProjectedTimelineV1,
    /// Keyed by `thread_id`.
    pub open_threads: BTreeMap<String, ProjectedThreadV1>,
    pub uncertain_orderings: Vec<OrderingUncertaintyV1>,
    pub precondition_anomalies: Vec<ReducerAnomalyV1>,
    pub invalidated_scene_suffixes: Vec<CanonPositionV1>,
    pub visible_event_ids: Vec<String>,
}

impl CanonStateViewV1 {
    pub fn entity(&self, subject_id: &str) -> Option<&ProjectedEntityStateV1> {
        self.entities.get(subject_id)
    }

    /// Missing state is always `LifeStatus::Unknown`, never assumed `Alive`.
    pub fn life_status(&self, subject_id: &str) -> LifeStatus {
        self.entities
            .get(subject_id)
            .and_then(|e| e.life_status.value)
            .unwrap_or(LifeStatus::Unknown)
    }

    /// Missing location is `None` (unknown).
    pub fn location(&self, subject_id: &str) -> Option<&str> {
        self.entities
            .get(subject_id)
            .and_then(|e| e.location_id.value.as_deref())
    }

    pub fn active_injuries(&self, subject_id: &str) -> BTreeMap<&str, &ProjectedInjuryV1> {
        self.entities
            .get(subject_id)
            .map(|e| {
                e.injuries
                    .iter()
                    .filter(|(_, inj)| inj.status == InjuryStatus::Active)
                    .map(|(k, v)| (k.as_str(), v))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn holds_item(
        &self,
        subject_id: &str,
        item_id: &str,
        custody: CustodyKind,
    ) -> Option<bool> {
        let entity = self.entities.get(subject_id)?;
        let key = possession_slot_key(item_id, custody);
        entity.possessions.get(&key).map(|p| p.held)
    }

    pub fn quantity(&self, subject_id: &str, measure: &str) -> Option<&ProjectedQuantityV1> {
        let key = quantity_slot_key(subject_id, measure);
        self.quantities.get(&key)
    }

    /// Missing quantity or delta-only stream without an opening balance returns `None`, never `"0"`.
    pub fn quantity_balance(&self, subject_id: &str, measure: &str) -> Option<&str> {
        self.quantity(subject_id, measure)
            .and_then(|q| q.balance.as_deref())
    }

    pub fn relationship(
        &self,
        subject_id: &str,
        other_id: &str,
    ) -> Option<&ProjectedRelationshipV1> {
        self.relationships
            .get(&relationship_slot_key(subject_id, other_id))
    }

    pub fn knowledge_status(&self, knower_id: &str, proposition_id: &str) -> KnowledgeStatus {
        self.knowledge
            .get(proposition_id)
            .and_then(|k| k.knowers.get(knower_id))
            .map(|edge| edge.status)
            .unwrap_or(KnowledgeStatus::Unknown)
    }
}

pub fn relationship_slot_key(subject_id: &str, other_id: &str) -> String {
    format!("{subject_id}->{other_id}")
}

pub fn quantity_slot_key(subject_id: &str, measure: &str) -> String {
    format!("{subject_id}:{}", key_text(measure))
}

pub fn possession_slot_key(item_id: &str, custody: CustodyKind) -> String {
    let kind = match custody {
        CustodyKind::Owner => "owner",
        CustodyKind::Custodian => "custodian",
    };
    format!("{item_id}:{kind}")
}

pub fn position_slot_key(pos: CanonPositionV1) -> String {
    format!(
        "{}:{}:{}",
        pos.book_number, pos.chapter_number, pos.scene_order
    )
}

fn push_unique_id(list: &mut Vec<String>, id: &str) {
    if !list.iter().any(|existing| existing == id) {
        list.push(id.to_string());
    }
}

/// Compute the active decision sequences on a single branch at `decision_watermark`,
/// accounting for any `WatermarkRestore` decisions appended at or before `decision_watermark`.
fn active_branch_decisions_at_watermark(
    branch_id: &str,
    events: &[CanonEventV1],
    decision_watermark: i64,
) -> BTreeSet<i64> {
    let mut decisions: BTreeMap<i64, Option<i64>> = BTreeMap::new();
    for event in events {
        if event.branch_id != branch_id || event.decision_seq > decision_watermark {
            continue;
        }
        let restore_target = match &event.payload {
            CanonEventPayloadV1::WatermarkRestore { restore } => Some(restore.target_decision_seq),
            _ => None,
        };
        decisions
            .entry(event.decision_seq)
            .and_modify(|existing| {
                if restore_target.is_some() {
                    *existing = restore_target;
                }
            })
            .or_insert(restore_target);
    }

    let mut history_at_seq: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    let mut current_active = BTreeSet::new();

    for (seq, maybe_restore_target) in decisions {
        if let Some(target_seq) = maybe_restore_target {
            current_active = history_at_seq
                .range(..=target_seq)
                .next_back()
                .map(|(_, active)| active.clone())
                .unwrap_or_default();
            current_active.insert(seq);
        } else {
            current_active.insert(seq);
        }
        history_at_seq.insert(seq, current_active.clone());
    }

    current_active
}

fn event_matches_legacy_whole_record_mask(
    event: &CanonEventV1,
    record_kind: &str,
    subject_id: &str,
    natural_key: &str,
) -> bool {
    if let Some(scope) = &event.legacy_record_scope {
        return scope.record_kind == record_kind && scope.natural_key == natural_key;
    }
    if event.subject_key() != subject_id {
        return false;
    }
    match record_kind {
        "character_state" => matches!(
            &event.payload,
            CanonEventPayloadV1::Assertion {
                assertion: ClaimAssertion::LifeStatus { .. }
                    | ClaimAssertion::LocatedAt { .. }
                    | ClaimAssertion::Move { .. }
                    | ClaimAssertion::Injury { .. }
                    | ClaimAssertion::Attribute { .. },
                ..
            }
        ),
        "canonical_fact" | "world_state" => matches!(
            &event.payload,
            CanonEventPayloadV1::Assertion {
                assertion: ClaimAssertion::Attribute { .. } | ClaimAssertion::LocatedAt { .. },
                ..
            }
        ),
        _ => false,
    }
}

fn is_delta_event(event: &CanonEventV1) -> bool {
    matches!(
        &event.payload,
        CanonEventPayloadV1::Assertion {
            assertion: ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                ..
            } | ClaimAssertion::Relationship {
                operation: RelationshipOperation::Delta,
                ..
            },
            ..
        }
    )
}

fn mask_matches_event_target(mask: &StreamCoverageMaskV1, event: &CanonEventV1) -> bool {
    match &mask.target {
        CoverageMaskTargetV1::AncestorEvent { event_id } => {
            &event.event_id == event_id
                || event
                    .merge_provenance
                    .as_ref()
                    .is_some_and(|mp| &mp.source_event_id == event_id)
        }
        CoverageMaskTargetV1::Stream { stream } => {
            event.semantic_stream_key().as_ref() == Some(stream)
        }
        CoverageMaskTargetV1::LegacyWholeRecord {
            record_kind,
            subject_id,
            natural_key,
        } => event_matches_legacy_whole_record_mask(event, record_kind, subject_id, natural_key),
    }
}

fn is_withdrawal_only_event(event: &CanonEventV1) -> bool {
    event.policy_action == CanonPolicyAction::Withdraw
        || (event.withdraws_event_id.is_some() && event.supersedes_event_id.is_none())
}

fn event_matches_id_or_merged_source(event: &CanonEventV1, target_id: &str) -> bool {
    event.event_id == target_id
        || event
            .merge_provenance
            .as_ref()
            .is_some_and(|mp| mp.source_event_id == target_id)
}

fn event_matches_revision_or_merged_source(event: &CanonEventV1, revision_id: &str) -> bool {
    event.source_revision_id == revision_id
        || event
            .merge_provenance
            .as_ref()
            .and_then(|mp| mp.source_revision_id.as_deref())
            .is_some_and(|src_rev| src_rev == revision_id)
}

fn active_lineage_events_at_watermark<'a>(
    branch_id: &str,
    lineage: &[String],
    events: &'a [CanonEventV1],
    tier: CanonTier,
    decision_watermark: Option<i64>,
) -> (Vec<String>, i64, i64, Vec<(usize, &'a CanonEventV1)>) {
    let effective_lineage: Vec<String> = if lineage.is_empty() {
        vec![branch_id.to_string()]
    } else {
        lineage.to_vec()
    };
    let branch_depths: BTreeMap<&str, usize> = effective_lineage
        .iter()
        .enumerate()
        .map(|(idx, b)| (b.as_str(), idx))
        .collect();

    let max_decision_seq = decision_watermark
        .unwrap_or_else(|| events.iter().map(|e| e.decision_seq).max().unwrap_or(0));
    let max_event_seq = events
        .iter()
        .filter(|e| e.decision_seq <= max_decision_seq)
        .map(|e| e.append_seq)
        .max()
        .unwrap_or(0);

    let mut active_decisions_by_branch: BTreeMap<&str, BTreeSet<i64>> = BTreeMap::new();
    for b in &effective_lineage {
        active_decisions_by_branch.insert(
            b.as_str(),
            active_branch_decisions_at_watermark(b, events, max_decision_seq),
        );
    }

    let mut active_lineage_events: Vec<(usize, &CanonEventV1)> = Vec::new();
    for event in events {
        let Some(&depth) = branch_depths.get(event.branch_id.as_str()) else {
            continue;
        };
        if event.decision_seq > max_decision_seq {
            continue;
        }
        if !active_decisions_by_branch
            .get(event.branch_id.as_str())
            .is_some_and(|set| set.contains(&event.decision_seq))
        {
            continue;
        }
        if tier == CanonTier::Ratified && event.tier != CanonTier::Ratified {
            continue;
        }
        active_lineage_events.push((depth, event));
    }

    (
        effective_lineage,
        max_decision_seq,
        max_event_seq,
        active_lineage_events,
    )
}

/// Fold visible canon events along `lineage` (`[child, parent, ..., main]`, never
/// siblings) through `cursor` at `tier` and `decision_watermark`.
pub fn state_at(
    branch_id: &str,
    lineage: &[String],
    events: &[CanonEventV1],
    cursor: &CanonCursorV1,
    tier: CanonTier,
    decision_watermark: Option<i64>,
) -> Result<CanonStateViewV1, CanonAppendError> {
    cursor.validate()?;
    let (effective_lineage, max_decision_seq, max_event_seq, active_lineage_events) =
        active_lineage_events_at_watermark(branch_id, lineage, events, tier, decision_watermark);

    // 3. Collect control state from active events:
    //    - Withdrawn / superseded event IDs
    //    - Source invalidations (scene revisions / remines)
    //    - Scene placement & visibility overrides (most specific branch depth wins)
    //    - Stream coverage masks
    let mut suppressed_event_ids: Vec<(usize, i64, i64, String)> = Vec::new();
    let mut invalidated_revisions: Vec<(usize, i64, i64, Option<String>, String)> = Vec::new();
    let mut scene_visibility_by_id: BTreeMap<String, (usize, ScenePlacementVisibilityV1)> =
        BTreeMap::new();
    let mut masks_by_depth: Vec<(usize, i64, i64, bool, StreamCoverageMaskV1)> = Vec::new();

    // Sort active events by (depth DESC, decision_seq ASC, append_seq ASC) so newer
    // scene visibility mappings at the same depth override older ones and child
    // depth overrides ancestor depth.
    let mut control_scan = active_lineage_events.clone();
    control_scan
        .sort_by_key(|(depth, e)| (std::cmp::Reverse(*depth), e.decision_seq, e.append_seq));

    for &(depth, event) in &control_scan {
        if let Some(withdrawn) = &event.withdraws_event_id {
            suppressed_event_ids.push((
                depth,
                event.decision_seq,
                event.append_seq,
                withdrawn.clone(),
            ));
        }
        if let Some(superseded) = &event.supersedes_event_id {
            suppressed_event_ids.push((
                depth,
                event.decision_seq,
                event.append_seq,
                superseded.clone(),
            ));
        }
        match &event.payload {
            CanonEventPayloadV1::SourceInvalidation { invalidation } => {
                for rev_id in &invalidation.invalidated_revision_ids {
                    invalidated_revisions.push((
                        depth,
                        event.decision_seq,
                        event.append_seq,
                        invalidation.scene_id.clone(),
                        rev_id.clone(),
                    ));
                }
                for ev_id in &invalidation.invalidated_event_ids {
                    suppressed_event_ids.push((
                        depth,
                        event.decision_seq,
                        event.append_seq,
                        ev_id.clone(),
                    ));
                }
            }
            CanonEventPayloadV1::SceneVisibility { visibility } => {
                scene_visibility_by_id
                    .insert(visibility.scene_id.clone(), (depth, visibility.clone()));
            }
            CanonEventPayloadV1::CoverageMask { mask } => {
                masks_by_depth.push((
                    depth,
                    event.decision_seq,
                    event.append_seq,
                    event.provenance == CanonProvenanceKind::Merged,
                    mask.clone(),
                ));
            }
            CanonEventPayloadV1::Assertion { .. }
            | CanonEventPayloadV1::WatermarkRestore { .. } => {}
        }
    }

    let mut invalidated_scene_suffixes: Vec<CanonPositionV1> = scene_visibility_by_id
        .values()
        .map(|(_, v)| v.invalidated_from)
        .collect();
    invalidated_scene_suffixes.sort();
    invalidated_scene_suffixes.dedup();

    // 4. Filter assertion events by withdrawals, source invalidations, scene
    //    visibility, cursor position, and child-over-ancestor coverage masks.
    let mut uncertain_orderings: Vec<OrderingUncertaintyV1> = Vec::new();
    let mut visible_events: Vec<(usize, Option<CanonPositionV1>, &CanonEventV1)> = Vec::new();

    for (depth, event) in active_lineage_events {
        if !matches!(event.payload, CanonEventPayloadV1::Assertion { .. })
            || is_withdrawal_only_event(event)
        {
            continue;
        }
        // Suppressed by a more-specific branch (`sup_depth < depth`) or a later
        // control/withdrawal event on the same branch.
        if suppressed_event_ids
            .iter()
            .any(|(sup_depth, sup_dec, sup_app, id)| {
                (*sup_depth < depth
                    || (*sup_depth == depth
                        && (event.decision_seq, event.append_seq) < (*sup_dec, *sup_app)))
                    && event_matches_id_or_merged_source(event, id)
            })
        {
            continue;
        }
        if invalidated_revisions.iter().any(
            |(inv_depth, inv_dec, inv_app, scene_filter, rev_id)| {
                (*inv_depth < depth
                    || (*inv_depth == depth
                        && (event.decision_seq, event.append_seq) < (*inv_dec, *inv_app)))
                    && event_matches_revision_or_merged_source(event, rev_id)
                    && scene_filter
                        .as_ref()
                        .is_none_or(|s| event.scene_id.as_ref() == Some(s))
            },
        ) {
            continue;
        }

        // Apply scene visibility / placement override.
        let mut effective_pos = event.effective_position;
        if let Some(scene_id) = &event.scene_id
            && let Some((vis_depth, vis)) = scene_visibility_by_id.get(scene_id)
            && *vis_depth <= depth
        {
            if !vis.visible {
                continue;
            }
            if let Some(override_pos) = vis.position_override {
                effective_pos = Some(override_pos);
            }
        }

        // Check cursor bounds.
        if let Some(cursor_pos) = cursor.position
            && let Some(ev_pos) = effective_pos
        {
            if ev_pos > cursor_pos {
                continue;
            }
            if ev_pos == cursor_pos
                && let Some(cursor_step) = cursor.within_scene_order
            {
                match event.within_scene_order {
                    Some(ev_step) if ev_step > cursor_step => continue,
                    Some(_) => {}
                    None => {
                        uncertain_orderings.push(OrderingUncertaintyV1 {
                            position: ev_pos,
                            stream_description: format!(
                                "{}:{}",
                                event.subject_key(),
                                event.predicate
                            ),
                            event_ids: vec![event.event_id.clone()],
                            reason: "event in cursor scene has unknown within_scene_order relative to sub-scene cursor".into(),
                        });
                    }
                }
            }
        }

        if !is_assertion_visible_at_position(
            depth,
            effective_pos,
            event,
            effective_lineage.len(),
            &masks_by_depth,
            cursor.position,
            cursor.position.is_none(),
        ) {
            continue;
        }

        visible_events.push((depth, effective_pos, event));
    }

    // 5. Sort visible events deterministically:
    //    (effective_position [None=baseline first], within_scene_order [Some(n) before None],
    //     lineage depth DESC [ancestor before child at identical step], decision_seq ASC,
    //     append_seq ASC, event_id ASC).
    visible_events.sort_by(|(depth_a, pos_a, ev_a), (depth_b, pos_b, ev_b)| {
        reducer_event_cmp(*depth_a, *pos_a, ev_a, *depth_b, *pos_b, ev_b)
    });

    let mut view = CanonStateViewV1 {
        branch_id: branch_id.to_string(),
        lineage: effective_lineage,
        cursor: cursor.clone(),
        tier,
        decision_watermark: max_decision_seq,
        event_watermark: max_event_seq,
        reducer_version: CANON_REDUCER_VERSION,
        entities: BTreeMap::new(),
        relationships: BTreeMap::new(),
        quantities: BTreeMap::new(),
        knowledge: BTreeMap::new(),
        rules: BTreeMap::new(),
        timeline: ProjectedTimelineV1 {
            scene_clocks: BTreeMap::new(),
            latest_clock: None,
            source_event_ids: Vec::new(),
            reducer_version: CANON_REDUCER_VERSION,
        },
        open_threads: BTreeMap::new(),
        uncertain_orderings,
        precondition_anomalies: Vec::new(),
        invalidated_scene_suffixes,
        visible_event_ids: Vec::new(),
    };

    // Track same-scene events per semantic stream to flag missing within_scene_order
    // when multiple non-commuting events occur in the same scene.
    type SameSceneStreamEvents = BTreeMap<(CanonPositionV1, String), Vec<(Option<u32>, String)>>;
    let mut same_scene_stream_events: SameSceneStreamEvents = BTreeMap::new();

    for (_depth, effective_pos, event) in &visible_events {
        view.visible_event_ids.push(event.event_id.clone());
        if let Some(pos) = *effective_pos
            && let Some(stream_key) = event.semantic_stream_key()
        {
            let stream_desc = serde_json::to_string(&stream_key).unwrap_or_default();
            let entry = same_scene_stream_events
                .entry((pos, stream_desc.clone()))
                .or_default();
            if !entry.is_empty()
                && (event.within_scene_order.is_none() || entry.iter().any(|(s, _)| s.is_none()))
            {
                let mut ids: Vec<String> = entry.iter().map(|(_, id)| id.clone()).collect();
                ids.push(event.event_id.clone());
                view.uncertain_orderings.push(OrderingUncertaintyV1 {
                    position: pos,
                    stream_description: stream_desc,
                    event_ids: ids,
                    reason: "multiple events on the same stream in one scene without complete within_scene_order".into(),
                });
            }
            entry.push((event.within_scene_order, event.event_id.clone()));
        }
        apply_event_to_view(&mut view, event, *effective_pos)?;
    }

    Ok(view)
}

fn apply_event_to_view(
    view: &mut CanonStateViewV1,
    event: &CanonEventV1,
    effective_pos: Option<CanonPositionV1>,
) -> Result<(), CanonAppendError> {
    let CanonEventPayloadV1::Assertion {
        assertion,
        modality,
        temporal_scope,
        ..
    } = &event.payload
    else {
        return Ok(());
    };

    // Non-narration or non-current assertions (dialogue, belief, speculation,
    // dream, negated, flashback, future) do not mutate current narrative truth.
    if *modality != ClaimModality::Narration || *temporal_scope != TemporalScope::Current {
        return Ok(());
    }

    let subject_id = event.subject_key();

    match assertion {
        ClaimAssertion::LifeStatus { status } => {
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            entity.life_status.value = Some(*status);
            entity.life_status.effective_position = effective_pos;
            entity.life_status.within_scene_order = event.within_scene_order;
            push_unique_id(&mut entity.life_status.source_event_ids, &event.event_id);
            push_unique_id(&mut entity.source_event_ids, &event.event_id);
        }
        ClaimAssertion::Action { verb, mode } => {
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            entity.actions.push(ProjectedActionOccurrenceV1 {
                event_id: event.event_id.clone(),
                verb: key_text(verb),
                mode: *mode,
                position: effective_pos,
                within_scene_order: event.within_scene_order,
                in_world_time: event.in_world_time.clone(),
            });
            push_unique_id(&mut entity.source_event_ids, &event.event_id);
        }
        ClaimAssertion::LocatedAt { location } => {
            let loc_id = entity_key(location);
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            entity.location_id.value = Some(loc_id);
            entity.location_id.effective_position = effective_pos;
            entity.location_id.within_scene_order = event.within_scene_order;
            push_unique_id(&mut entity.location_id.source_event_ids, &event.event_id);
            push_unique_id(&mut entity.source_event_ids, &event.event_id);
        }
        ClaimAssertion::Move { from, to } => {
            let from_id = entity_key(from);
            let to_id = entity_key(to);
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            if let Some(current_loc) = &entity.location_id.value
                && current_loc != &from_id
            {
                view.precondition_anomalies.push(ReducerAnomalyV1 {
                    event_id: event.event_id.clone(),
                    code: CanonAppendErrorCode::Precondition,
                    detail: format!(
                        "move origin {from_id} does not match current location {current_loc}"
                    ),
                });
            }
            entity.location_id.value = Some(to_id);
            entity.location_id.effective_position = effective_pos;
            entity.location_id.within_scene_order = event.within_scene_order;
            push_unique_id(&mut entity.location_id.source_event_ids, &event.event_id);
            push_unique_id(&mut entity.source_event_ids, &event.event_id);
        }
        ClaimAssertion::SceneClock { clock } => {
            if let Some(pos) = effective_pos {
                let slot = position_slot_key(pos);
                let entry = view.timeline.scene_clocks.entry(slot).or_insert_with(|| {
                    ProjectedSceneClockV1 {
                        position: pos,
                        scene_id: event.scene_id.clone(),
                        clock: clock.clone(),
                        source_event_ids: Vec::new(),
                        reducer_version: CANON_REDUCER_VERSION,
                    }
                });
                entry.clock = clock.clone();
                entry.scene_id = event.scene_id.clone();
                push_unique_id(&mut entry.source_event_ids, &event.event_id);
                view.timeline.latest_clock = Some(entry.clone());
                push_unique_id(&mut view.timeline.source_event_ids, &event.event_id);
            }
        }
        ClaimAssertion::Possession {
            operation,
            item,
            counterparty,
            custody,
        } => {
            let item_id = entity_key(item);
            let slot_key = possession_slot_key(&item_id, *custody);
            let other_id = counterparty.as_ref().map(entity_key);
            match operation {
                PossessionOperation::Acquire => {
                    let entity = view
                        .entities
                        .entry(subject_id.clone())
                        .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
                    let entry = entity.possessions.entry(slot_key).or_insert_with(|| {
                        ProjectedPossessionV1 {
                            item_id: item_id.clone(),
                            custody: *custody,
                            held: true,
                            counterparty_id: other_id.clone(),
                            updated_at: effective_pos,
                            source_event_ids: Vec::new(),
                            reducer_version: CANON_REDUCER_VERSION,
                        }
                    });
                    entry.held = true;
                    entry.counterparty_id = other_id;
                    entry.updated_at = effective_pos;
                    push_unique_id(&mut entry.source_event_ids, &event.event_id);
                    push_unique_id(&mut entity.source_event_ids, &event.event_id);
                }
                PossessionOperation::Lose => {
                    let entity = view
                        .entities
                        .entry(subject_id.clone())
                        .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
                    if !entity
                        .possessions
                        .get(&slot_key)
                        .is_some_and(|existing| existing.held)
                    {
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!("cannot lose unheld possession {item_id}"),
                        });
                    }
                    let entry = entity.possessions.entry(slot_key).or_insert_with(|| {
                        ProjectedPossessionV1 {
                            item_id: item_id.clone(),
                            custody: *custody,
                            held: false,
                            counterparty_id: other_id.clone(),
                            updated_at: effective_pos,
                            source_event_ids: Vec::new(),
                            reducer_version: CANON_REDUCER_VERSION,
                        }
                    });
                    entry.held = false;
                    entry.counterparty_id = other_id;
                    entry.updated_at = effective_pos;
                    push_unique_id(&mut entry.source_event_ids, &event.event_id);
                    push_unique_id(&mut entity.source_event_ids, &event.event_id);
                }
                PossessionOperation::Transfer => {
                    let sender_holds = view
                        .entities
                        .get(&subject_id)
                        .and_then(|e| e.possessions.get(&slot_key))
                        .is_some_and(|p| p.held);
                    if !sender_holds {
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!(
                                "cannot transfer possession {item_id} when sender {subject_id} does not hold it"
                            ),
                        });
                    } else {
                        let sender = view
                            .entities
                            .entry(subject_id.clone())
                            .or_insert_with(|| ProjectedEntityStateV1::new(subject_id.clone()));
                        let sender_entry = sender
                            .possessions
                            .entry(slot_key.clone())
                            .or_insert_with(|| ProjectedPossessionV1 {
                                item_id: item_id.clone(),
                                custody: *custody,
                                held: false,
                                counterparty_id: other_id.clone(),
                                updated_at: effective_pos,
                                source_event_ids: Vec::new(),
                                reducer_version: CANON_REDUCER_VERSION,
                            });
                        sender_entry.held = false;
                        sender_entry.counterparty_id = other_id.clone();
                        sender_entry.updated_at = effective_pos;
                        push_unique_id(&mut sender_entry.source_event_ids, &event.event_id);
                        push_unique_id(&mut sender.source_event_ids, &event.event_id);

                        if let Some(receiver_id) = other_id {
                            let receiver = view
                                .entities
                                .entry(receiver_id.clone())
                                .or_insert_with(|| ProjectedEntityStateV1::new(receiver_id));
                            let recv_entry =
                                receiver.possessions.entry(slot_key).or_insert_with(|| {
                                    ProjectedPossessionV1 {
                                        item_id,
                                        custody: *custody,
                                        held: true,
                                        counterparty_id: Some(subject_id),
                                        updated_at: effective_pos,
                                        source_event_ids: Vec::new(),
                                        reducer_version: CANON_REDUCER_VERSION,
                                    }
                                });
                            recv_entry.held = true;
                            recv_entry.updated_at = effective_pos;
                            push_unique_id(&mut recv_entry.source_event_ids, &event.event_id);
                            push_unique_id(&mut receiver.source_event_ids, &event.event_id);
                        }
                    }
                }
            }
        }
        ClaimAssertion::Injury {
            operation,
            injury_id,
            body_site,
            severity,
        } => {
            let norm_id = injury_id.trim().to_string();
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            match operation {
                InjuryOperation::Add => {
                    let entry = entity.injuries.entry(norm_id.clone()).or_insert_with(|| {
                        ProjectedInjuryV1 {
                            injury_id: norm_id,
                            body_site: body_site.trim().to_string(),
                            severity: *severity,
                            status: InjuryStatus::Active,
                            acquired_at: effective_pos,
                            healed_at: None,
                            source_event_ids: Vec::new(),
                            reducer_version: CANON_REDUCER_VERSION,
                        }
                    });
                    entry.body_site = body_site.trim().to_string();
                    entry.severity = *severity;
                    entry.status = InjuryStatus::Active;
                    if entry.acquired_at.is_none() {
                        entry.acquired_at = effective_pos;
                    }
                    entry.healed_at = None;
                    push_unique_id(&mut entry.source_event_ids, &event.event_id);
                    push_unique_id(&mut entity.source_event_ids, &event.event_id);
                }
                InjuryOperation::Heal => {
                    if let Some(entry) = entity.injuries.get_mut(&norm_id) {
                        entry.status = InjuryStatus::Healed;
                        entry.healed_at = effective_pos;
                        push_unique_id(&mut entry.source_event_ids, &event.event_id);
                        push_unique_id(&mut entity.source_event_ids, &event.event_id);
                    } else {
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!("cannot heal unknown injury {norm_id}"),
                        });
                    }
                }
            }
        }
        ClaimAssertion::Quantity {
            operation,
            measure,
            amount,
            unit,
            transaction_id,
        } => {
            let norm_measure = key_text(measure);
            let norm_unit = unit.trim().to_string();
            let parsed_amount = ExactDecimal::parse(amount)?;
            let slot_key = quantity_slot_key(&subject_id, &norm_measure);
            let entry = view
                .quantities
                .entry(slot_key)
                .or_insert_with(|| ProjectedQuantityV1 {
                    subject_id: subject_id.clone(),
                    measure: norm_measure,
                    unit: norm_unit.clone(),
                    opening_balance: None,
                    balance: None,
                    applied_transactions: BTreeSet::new(),
                    unapplied_deltas_without_opening_balance: 0,
                    source_event_ids: Vec::new(),
                    reducer_version: CANON_REDUCER_VERSION,
                });
            match operation {
                QuantityOperation::Set => {
                    let canonical = parsed_amount.to_canonical_string();
                    entry.unit = norm_unit;
                    if entry.opening_balance.is_none() {
                        entry.opening_balance = Some(canonical.clone());
                    }
                    entry.balance = Some(canonical);
                    push_unique_id(&mut entry.source_event_ids, &event.event_id);
                }
                QuantityOperation::Delta => {
                    if entry.unit != norm_unit {
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::UnitMismatch,
                            detail: format!(
                                "quantity delta unit {} does not match stream unit {}",
                                norm_unit, entry.unit
                            ),
                        });
                    } else if let Some(current_str) = &entry.balance {
                        if let Some(tx_id) = transaction_id
                            && !entry.applied_transactions.insert(tx_id.clone())
                        {
                            // Duplicate transaction_id for the same participant/measure is skipped.
                            return Ok(());
                        }
                        let current = ExactDecimal::parse(current_str)?;
                        let updated = current.checked_add(parsed_amount)?;
                        entry.balance = Some(updated.to_canonical_string());
                        push_unique_id(&mut entry.source_event_ids, &event.event_id);
                    } else {
                        // Missing opening balance stays `None` (unknown), never invented as 0.
                        entry.unapplied_deltas_without_opening_balance += 1;
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!(
                                "quantity delta on {}:{} requires an opening balance",
                                subject_id, entry.measure
                            ),
                        });
                    }
                }
            }
        }
        ClaimAssertion::Knowledge {
            operation,
            proposition,
            audience,
            source,
        } => {
            let prop_id = entity_key(proposition);
            let k_proj =
                view.knowledge
                    .entry(prop_id.clone())
                    .or_insert_with(|| ProjectedKnowledgeV1 {
                        proposition_id: prop_id,
                        knowers: BTreeMap::new(),
                        circle_of_trust: BTreeSet::new(),
                        disclosures: Vec::new(),
                        source_event_ids: Vec::new(),
                        reducer_version: CANON_REDUCER_VERSION,
                    });
            push_unique_id(&mut k_proj.source_event_ids, &event.event_id);

            match operation {
                KnowledgeOperation::Learn => {
                    let edge = k_proj.knowers.entry(subject_id.clone()).or_insert_with(|| {
                        ProjectedKnowerEdgeV1 {
                            knower_id: subject_id.clone(),
                            status: KnowledgeStatus::Known,
                            learned_at: effective_pos,
                            learned_within_scene_order: event.within_scene_order,
                            learned_in_world_time: event.in_world_time.clone(),
                            source_entity_id: source.as_ref().map(entity_key),
                            ordering_uncertain: false,
                            uses: Vec::new(),
                            source_event_ids: Vec::new(),
                        }
                    });
                    // Check if there was already a same-scene Use before this Learn where
                    // within_scene_order was missing on either event.
                    for prior_use in &mut edge.uses {
                        if prior_use.position == effective_pos
                            && (prior_use.within_scene_order.is_none()
                                || event.within_scene_order.is_none())
                        {
                            prior_use.ordering_uncertain = true;
                            edge.ordering_uncertain = true;
                        }
                    }
                    edge.status = KnowledgeStatus::Known;
                    if edge.learned_at.is_none() {
                        edge.learned_at = effective_pos;
                        edge.learned_within_scene_order = event.within_scene_order;
                        edge.learned_in_world_time = event.in_world_time.clone();
                    }
                    if edge.source_entity_id.is_none() {
                        edge.source_entity_id = source.as_ref().map(entity_key);
                    }
                    push_unique_id(&mut edge.source_event_ids, &event.event_id);
                    k_proj.circle_of_trust.insert(subject_id);
                }
                KnowledgeOperation::Forget => {
                    let edge = k_proj.knowers.entry(subject_id.clone()).or_insert_with(|| {
                        ProjectedKnowerEdgeV1 {
                            knower_id: subject_id.clone(),
                            status: KnowledgeStatus::Forgotten,
                            learned_at: None,
                            learned_within_scene_order: None,
                            learned_in_world_time: None,
                            source_entity_id: None,
                            ordering_uncertain: false,
                            uses: Vec::new(),
                            source_event_ids: Vec::new(),
                        }
                    });
                    edge.status = KnowledgeStatus::Forgotten;
                    push_unique_id(&mut edge.source_event_ids, &event.event_id);
                    k_proj.circle_of_trust.remove(&subject_id);
                }
                KnowledgeOperation::Use => {
                    let edge = k_proj.knowers.entry(subject_id.clone()).or_insert_with(|| {
                        ProjectedKnowerEdgeV1 {
                            knower_id: subject_id,
                            status: KnowledgeStatus::Unknown,
                            learned_at: None,
                            learned_within_scene_order: None,
                            learned_in_world_time: None,
                            source_entity_id: None,
                            ordering_uncertain: false,
                            uses: Vec::new(),
                            source_event_ids: Vec::new(),
                        }
                    });
                    let same_scene_uncertain = edge.learned_at.is_some()
                        && edge.learned_at == effective_pos
                        && (edge.learned_within_scene_order.is_none()
                            || event.within_scene_order.is_none());
                    if same_scene_uncertain {
                        edge.ordering_uncertain = true;
                    }
                    edge.uses.push(ProjectedKnowledgeUseV1 {
                        event_id: event.event_id.clone(),
                        position: effective_pos,
                        within_scene_order: event.within_scene_order,
                        status_at_use: edge.status,
                        ordering_uncertain: same_scene_uncertain,
                    });
                    push_unique_id(&mut edge.source_event_ids, &event.event_id);
                }
                KnowledgeOperation::Disclose => {
                    let audience_ids: Vec<String> = audience.iter().map(entity_key).collect();
                    k_proj.disclosures.push(ProjectedDisclosureV1 {
                        event_id: event.event_id.clone(),
                        speaker_id: subject_id.clone(),
                        audience_ids: audience_ids.clone(),
                        position: effective_pos,
                        within_scene_order: event.within_scene_order,
                        in_world_time: event.in_world_time.clone(),
                    });
                    for listener_id in audience_ids {
                        let edge = k_proj
                            .knowers
                            .entry(listener_id.clone())
                            .or_insert_with(|| ProjectedKnowerEdgeV1 {
                                knower_id: listener_id.clone(),
                                status: KnowledgeStatus::Known,
                                learned_at: effective_pos,
                                learned_within_scene_order: event.within_scene_order,
                                learned_in_world_time: event.in_world_time.clone(),
                                source_entity_id: Some(subject_id.clone()),
                                ordering_uncertain: false,
                                uses: Vec::new(),
                                source_event_ids: Vec::new(),
                            });
                        edge.status = KnowledgeStatus::Known;
                        if edge.learned_at.is_none() {
                            edge.learned_at = effective_pos;
                            edge.learned_within_scene_order = event.within_scene_order;
                            edge.learned_in_world_time = event.in_world_time.clone();
                        }
                        push_unique_id(&mut edge.source_event_ids, &event.event_id);
                        k_proj.circle_of_trust.insert(listener_id);
                    }
                }
            }
        }
        ClaimAssertion::Attribute {
            operation,
            key,
            value,
            nature,
        } => {
            let norm_key = key_text(key);
            let normalized_val = match normalized_assertion(assertion) {
                ClaimAssertion::Attribute { value, .. } => value,
                _ => value.clone(),
            };
            let entity = view
                .entities
                .entry(subject_id.clone())
                .or_insert_with(|| ProjectedEntityStateV1::new(subject_id));
            let attr = entity
                .attributes
                .entry(norm_key.clone())
                .or_insert_with(|| ProjectedAttributeV1 {
                    key: norm_key,
                    nature: *nature,
                    scalar_value: None,
                    members: BTreeMap::new(),
                    effective_position: effective_pos,
                    source_event_ids: Vec::new(),
                    reducer_version: CANON_REDUCER_VERSION,
                });
            attr.nature = *nature;
            attr.effective_position = effective_pos;
            match operation {
                AttributeOperation::Set => {
                    attr.scalar_value = Some(normalized_val);
                    push_unique_id(&mut attr.source_event_ids, &event.event_id);
                    push_unique_id(&mut entity.source_event_ids, &event.event_id);
                }
                AttributeOperation::Add => {
                    let member_key = scalar_member_key(&normalized_val);
                    attr.members.insert(member_key, normalized_val);
                    push_unique_id(&mut attr.source_event_ids, &event.event_id);
                    push_unique_id(&mut entity.source_event_ids, &event.event_id);
                }
                AttributeOperation::Remove => {
                    let member_key = scalar_member_key(&normalized_val);
                    if attr.members.remove(&member_key).is_none() {
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!("cannot remove absent attribute member {member_key}"),
                        });
                    } else {
                        push_unique_id(&mut attr.source_event_ids, &event.event_id);
                        push_unique_id(&mut entity.source_event_ids, &event.event_id);
                    }
                }
            }
        }
        ClaimAssertion::Relationship {
            operation,
            other,
            trust,
            tension,
        } => {
            let other_id = entity_key(other);
            let slot_key = relationship_slot_key(&subject_id, &other_id);
            let rel =
                view.relationships
                    .entry(slot_key)
                    .or_insert_with(|| ProjectedRelationshipV1 {
                        subject_id: subject_id.clone(),
                        other_id,
                        opening_trust: None,
                        opening_tension: None,
                        accumulated_trust_delta: 0,
                        accumulated_tension_delta: 0,
                        resolved_trust: None,
                        resolved_tension: None,
                        scene_delta_ledger: BTreeMap::new(),
                        source_event_ids: Vec::new(),
                        reducer_version: CANON_REDUCER_VERSION,
                    });
            match operation {
                RelationshipOperation::Observation => {
                    rel.opening_trust = Some(*trust);
                    rel.opening_tension = Some(*tension);
                    rel.accumulated_trust_delta = 0;
                    rel.accumulated_tension_delta = 0;
                    rel.resolved_trust = Some(*trust);
                    rel.resolved_tension = Some(*tension);
                    push_unique_id(&mut rel.source_event_ids, &event.event_id);
                }
                RelationshipOperation::Delta => {
                    rel.accumulated_trust_delta += *trust;
                    rel.accumulated_tension_delta += *tension;
                    if let Some(scene_id) = &event.scene_id {
                        let entry = rel
                            .scene_delta_ledger
                            .entry(scene_id.clone())
                            .or_insert((0, 0));
                        entry.0 += *trust;
                        entry.1 += *tension;
                    }
                    if let (Some(open_tr), Some(open_te)) = (rel.opening_trust, rel.opening_tension)
                    {
                        rel.resolved_trust = Some(open_tr + rel.accumulated_trust_delta);
                        rel.resolved_tension = Some(open_te + rel.accumulated_tension_delta);
                    } else {
                        // Missing opening balance stays `None`, never invented as (0, 0).
                        view.precondition_anomalies.push(ReducerAnomalyV1 {
                            event_id: event.event_id.clone(),
                            code: CanonAppendErrorCode::Precondition,
                            detail: format!(
                                "relationship delta on {}->{} has no opening observation",
                                rel.subject_id, rel.other_id
                            ),
                        });
                    }
                    push_unique_id(&mut rel.source_event_ids, &event.event_id);
                }
            }
        }
        ClaimAssertion::Promise { operation, thread } => {
            let thread_id = entity_key(thread);
            let proj = view
                .open_threads
                .entry(thread_id.clone())
                .or_insert_with(|| ProjectedThreadV1 {
                    thread_id,
                    status: PromiseThreadStatus::Unknown,
                    planted_at: None,
                    transitions: Vec::new(),
                    source_event_ids: Vec::new(),
                    reducer_version: CANON_REDUCER_VERSION,
                });
            proj.status = match operation {
                PromiseOperation::Plant => {
                    if proj.planted_at.is_none() {
                        proj.planted_at = effective_pos;
                    }
                    PromiseThreadStatus::Planted
                }
                PromiseOperation::Reinforce => PromiseThreadStatus::Reinforced,
                PromiseOperation::Payoff => PromiseThreadStatus::PaidOff,
                PromiseOperation::Abandon => PromiseThreadStatus::Abandoned,
            };
            proj.transitions.push(ProjectedPromiseTransitionV1 {
                event_id: event.event_id.clone(),
                subject_id,
                operation: *operation,
                position: effective_pos,
                within_scene_order: event.within_scene_order,
            });
            push_unique_id(&mut proj.source_event_ids, &event.event_id);
        }
        ClaimAssertion::Rule {
            operation,
            rule,
            anchor,
            preconditions,
            effects,
        } => {
            let rule_id = entity_key(rule);
            let proj =
                view.rules
                    .entry(rule_id.clone())
                    .or_insert_with(|| ProjectedRuleConstraintV1 {
                        rule_id,
                        defined: false,
                        preconditions: Vec::new(),
                        effects: Vec::new(),
                        anchors: BTreeSet::new(),
                        occurrences: Vec::new(),
                        source_event_ids: Vec::new(),
                        reducer_version: CANON_REDUCER_VERSION,
                    });
            if *operation == RuleOperation::Define {
                proj.defined = true;
                proj.preconditions = preconditions.clone();
                proj.effects = effects.clone();
            }
            if let Some(anc) = anchor {
                proj.anchors.insert(entity_key(anc));
            }
            proj.occurrences.push(ProjectedRuleOccurrenceV1 {
                event_id: event.event_id.clone(),
                actor_id: subject_id,
                operation: *operation,
                anchor_id: anchor.as_ref().map(entity_key),
                position: effective_pos,
                within_scene_order: event.within_scene_order,
            });
            push_unique_id(&mut proj.source_event_ids, &event.event_id);
        }
    }

    Ok(())
}

/// Validate a proposed batch of events before atomic decision+event append:
/// - checks schema version, identity, and position invariants
/// - enforces v1 auto-ratification policy guard (`disallowed_for_v1_auto_ratification`)
/// - validates transfer conservation across paired `Quantity` deltas sharing a `transaction_id`
/// - validates opening balances, units, and explicit set-member preconditions against `prior_state`
fn validate_event_batch_structure_and_conservation(
    actor: &str,
    batch: &[CanonEventV1],
) -> Result<(), CanonAppendError> {
    if batch.is_empty() {
        return Err(append_error(
            CanonAppendErrorCode::Precondition,
            "events",
            "canon decision requires at least one event",
        ));
    }
    if actor.trim().is_empty() {
        return Err(append_error(
            CanonAppendErrorCode::Identity,
            "actor",
            "decision actor must not be empty",
        ));
    }

    struct TxLeg {
        index: usize,
        subject_id: String,
        measure: String,
        unit: String,
        amount: ExactDecimal,
        position: Option<CanonPositionV1>,
    }
    let mut tx_groups: BTreeMap<String, Vec<TxLeg>> = BTreeMap::new();

    for (idx, event) in batch.iter().enumerate() {
        let with_idx = |mut err: CanonAppendError| {
            err.event_index = Some(idx);
            err
        };

        if event.schema_version != CANON_EVENT_SCHEMA_VERSION {
            return Err(with_idx(append_error(
                CanonAppendErrorCode::Schema,
                "schema_version",
                "unsupported canon event schema version",
            )));
        }
        if event.event_id.trim().is_empty()
            || event.project_id.trim().is_empty()
            || event.branch_id.trim().is_empty()
            || event.source_revision_id.trim().is_empty()
            || event.evidence_sha256.trim().is_empty()
        {
            return Err(with_idx(append_error(
                CanonAppendErrorCode::Identity,
                "event",
                "event identity fields must not be empty",
            )));
        }
        if let Some(pos) = event.effective_position {
            CanonPositionV1::new(pos.book_number, pos.chapter_number, pos.scene_order)
                .map_err(with_idx)?;
        }

        match &event.payload {
            CanonEventPayloadV1::CoverageMask { mask } => {
                mask.validate().map_err(with_idx)?;
            }
            CanonEventPayloadV1::SceneVisibility { visibility } => {
                if visibility.scene_id.trim().is_empty() {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Identity,
                        "scene_visibility.scene_id",
                        "scene_id must not be empty",
                    )));
                }
                CanonPositionV1::new(
                    visibility.invalidated_from.book_number,
                    visibility.invalidated_from.chapter_number,
                    visibility.invalidated_from.scene_order,
                )
                .map_err(with_idx)?;
                if let Some(pos) = visibility.position_override {
                    CanonPositionV1::new(pos.book_number, pos.chapter_number, pos.scene_order)
                        .map_err(with_idx)?;
                }
            }
            CanonEventPayloadV1::SourceInvalidation { invalidation } => {
                if invalidation.invalidated_revision_ids.is_empty()
                    && invalidation.invalidated_event_ids.is_empty()
                {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Precondition,
                        "source_invalidation",
                        "source invalidation must specify at least one revision or event id",
                    )));
                }
            }
            CanonEventPayloadV1::WatermarkRestore { restore } => {
                if restore.target_decision_seq < 0 || restore.target_event_seq < 0 {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Precondition,
                        "watermark_restore",
                        "watermark restore sequence must be non-negative",
                    )));
                }
            }
            CanonEventPayloadV1::Assertion {
                assertion,
                modality,
                temporal_scope,
                speaker,
                experiencer,
            } => {
                if event.tier == CanonTier::Ratified
                    && event.provenance != CanonProvenanceKind::LegacyUnverified
                    && assertion_has_unresolved_entity(
                        &event.subject,
                        assertion,
                        speaker,
                        experiencer,
                    )
                {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::UnresolvedEntity,
                        "entity_reference",
                        "ratified canon event requires resolved entity references",
                    )));
                }
                if *modality == ClaimModality::Narration
                    && *temporal_scope == TemporalScope::Current
                    && !is_withdrawal_only_event(event)
                    && let ClaimAssertion::Quantity {
                        operation,
                        measure,
                        amount,
                        unit,
                        transaction_id,
                    } = assertion
                {
                    let parsed = ExactDecimal::parse(amount).map_err(with_idx)?;
                    let norm_measure = key_text(measure);
                    let norm_unit = unit.trim().to_string();
                    if norm_unit.is_empty() || norm_measure.is_empty() {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::Schema,
                            "quantity",
                            "quantity requires nonempty measure and unit",
                        )));
                    }
                    if let Some(tx_id) = transaction_id {
                        if *operation != QuantityOperation::Delta {
                            return Err(with_idx(append_error(
                                CanonAppendErrorCode::Conservation,
                                "quantity.transaction_id",
                                "paired quantity transfer leg must use QuantityOperation::Delta",
                            )));
                        }
                        if parsed.is_zero() {
                            return Err(with_idx(append_error(
                                CanonAppendErrorCode::Conservation,
                                "quantity.amount",
                                "paired quantity transfer leg must have non-zero amount",
                            )));
                        }
                        tx_groups.entry(tx_id.clone()).or_default().push(TxLeg {
                            index: idx,
                            subject_id: event.subject_key(),
                            measure: norm_measure,
                            unit: norm_unit,
                            amount: parsed,
                            position: event.effective_position,
                        });
                    }
                }
            }
        }
    }

    // Validate transfer conservation for every `transaction_id` group in the batch.
    for (tx_id, legs) in tx_groups {
        let first_idx = legs[0].index;
        let with_first = |mut err: CanonAppendError| {
            err.event_index = Some(first_idx);
            err
        };
        if legs.len() < 2 {
            return Err(with_first(append_error(
                CanonAppendErrorCode::Conservation,
                "quantity.transaction_id",
                format!("quantity transfer {tx_id} requires at least two paired delta legs"),
            )));
        }
        let expected_measure = &legs[0].measure;
        let expected_unit = &legs[0].unit;
        let expected_position = legs[0].position;
        let mut total = ExactDecimal::zero();
        let mut distinct_subjects = BTreeSet::new();
        for leg in &legs {
            if &leg.measure != expected_measure {
                return Err(with_first(append_error(
                    CanonAppendErrorCode::Conservation,
                    "quantity.measure",
                    format!(
                        "quantity transfer {tx_id} mixes measures {} and {}",
                        expected_measure, leg.measure
                    ),
                )));
            }
            if &leg.unit != expected_unit {
                return Err(with_first(append_error(
                    CanonAppendErrorCode::UnitMismatch,
                    "quantity.unit",
                    format!(
                        "quantity transfer {tx_id} mixes units {} and {}",
                        expected_unit, leg.unit
                    ),
                )));
            }
            if leg.position != expected_position {
                return Err(with_first(append_error(
                    CanonAppendErrorCode::Conservation,
                    "quantity.position",
                    format!("quantity transfer {tx_id} legs must share effective position"),
                )));
            }
            distinct_subjects.insert(leg.subject_id.as_str());
            total = total.checked_add(leg.amount).map_err(with_first)?;
        }
        if distinct_subjects.len() < 2 {
            return Err(with_first(append_error(
                CanonAppendErrorCode::Conservation,
                "quantity.subject",
                format!("quantity transfer {tx_id} requires at least two distinct participants"),
            )));
        }
        if !total.is_zero() {
            return Err(with_first(append_error(
                CanonAppendErrorCode::Conservation,
                "quantity.amount",
                format!(
                    "quantity transfer {tx_id} violates conservation: net delta sum is {}",
                    total.to_canonical_string()
                ),
            )));
        }
    }

    Ok(())
}

fn validate_single_assertion_preconditions(
    actor: &str,
    action: CanonPolicyAction,
    idx: usize,
    event: &CanonEventV1,
    working_state: &CanonStateViewV1,
) -> Result<(), CanonAppendError> {
    let with_idx = |mut err: CanonAppendError| {
        err.event_index = Some(idx);
        err
    };
    let CanonEventPayloadV1::Assertion {
        assertion,
        modality,
        temporal_scope,
        speaker,
        experiencer,
    } = &event.payload
    else {
        return Ok(());
    };
    if is_withdrawal_only_event(event) {
        return Ok(());
    }

    let subject_id = event.subject_key();
    let prior_life = working_state
        .entities
        .get(&subject_id)
        .and_then(|e| e.life_status.value);
    let prior_invariant_differs = match assertion {
        ClaimAssertion::Attribute {
            key,
            value,
            nature: AttributeNature::Invariant,
            ..
        } => {
            let norm_key = key_text(key);
            let norm_val = match normalized_assertion(assertion) {
                ClaimAssertion::Attribute { value, .. } => value,
                _ => value.clone(),
            };
            working_state
                .entities
                .get(&subject_id)
                .and_then(|e| e.attributes.get(&norm_key))
                .and_then(|a| a.scalar_value.as_ref())
                .is_some_and(|existing| existing != &norm_val)
        }
        _ => false,
    };

    // Enforce v1 policy auto-ratification guard.
    if action == CanonPolicyAction::Ratify
        && is_policy_actor(actor)
        && let Some(reason) = disallowed_for_v1_auto_ratification(
            &event.subject,
            assertion,
            speaker,
            experiencer,
            prior_life,
            prior_invariant_differs,
        )
    {
        return Err(with_idx(append_error(
            CanonAppendErrorCode::PolicyGuard,
            "policy_action",
            reason,
        )));
    }

    // Precondition checks for current narrative assertions.
    if *modality == ClaimModality::Narration && *temporal_scope == TemporalScope::Current {
        match assertion {
            ClaimAssertion::Quantity {
                operation,
                measure,
                amount,
                unit,
                ..
            } => {
                let _parsed = ExactDecimal::parse(amount).map_err(with_idx)?;
                let norm_measure = key_text(measure);
                let norm_unit = unit.trim().to_string();
                if norm_unit.is_empty() || norm_measure.is_empty() {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Schema,
                        "quantity",
                        "quantity requires nonempty measure and unit",
                    )));
                }
                if *operation == QuantityOperation::Delta
                    && event.provenance != CanonProvenanceKind::LegacyUnverified
                {
                    let Some(existing) = working_state.quantity(&subject_id, &norm_measure) else {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::Precondition,
                            "quantity.opening_balance",
                            format!(
                                "quantity delta for {subject_id}:{norm_measure} requires an opening balance"
                            ),
                        )));
                    };
                    if existing.balance.is_none() {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::Precondition,
                            "quantity.opening_balance",
                            format!(
                                "quantity delta for {subject_id}:{norm_measure} requires an opening balance"
                            ),
                        )));
                    }
                    if existing.unit != norm_unit {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::UnitMismatch,
                            "quantity.unit",
                            format!(
                                "quantity delta unit {norm_unit} does not match opening unit {}",
                                existing.unit
                            ),
                        )));
                    }
                }
            }
            ClaimAssertion::Possession {
                operation,
                item,
                counterparty,
                custody,
            } => {
                let item_id = entity_key(item);
                if *operation == PossessionOperation::Transfer {
                    let Some(other) = counterparty else {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::Precondition,
                            "possession.counterparty",
                            "possession transfer requires a counterparty",
                        )));
                    };
                    if entity_key(other) == subject_id {
                        return Err(with_idx(append_error(
                            CanonAppendErrorCode::Precondition,
                            "possession.counterparty",
                            "possession transfer counterparty must differ from subject",
                        )));
                    }
                }
                if matches!(
                    operation,
                    PossessionOperation::Transfer | PossessionOperation::Lose
                ) && event.provenance != CanonProvenanceKind::LegacyUnverified
                    && working_state.holds_item(&subject_id, &item_id, *custody) != Some(true)
                {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Precondition,
                        "possession.held",
                        format!(
                            "{subject_id} does not currently hold item {item_id} ({custody:?})"
                        ),
                    )));
                }
            }
            ClaimAssertion::Injury {
                operation: InjuryOperation::Heal,
                injury_id,
                ..
            } if event.provenance != CanonProvenanceKind::LegacyUnverified => {
                let norm_id = injury_id.trim();
                let active = working_state
                    .entities
                    .get(&subject_id)
                    .and_then(|e| e.injuries.get(norm_id))
                    .is_some_and(|inj| inj.status == InjuryStatus::Active);
                if !active {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Precondition,
                        "injury.heal",
                        format!("cannot heal injury {norm_id} on {subject_id} when not active"),
                    )));
                }
            }
            ClaimAssertion::Attribute {
                operation: AttributeOperation::Remove,
                key,
                value,
                ..
            } if event.provenance != CanonProvenanceKind::LegacyUnverified => {
                let norm_key = key_text(key);
                let member_key = scalar_member_key(value);
                let present = working_state
                    .entities
                    .get(&subject_id)
                    .and_then(|e| e.attributes.get(&norm_key))
                    .is_some_and(|a| a.members.contains_key(&member_key));
                if !present {
                    return Err(with_idx(append_error(
                        CanonAppendErrorCode::Precondition,
                        "attribute.remove",
                        format!(
                            "cannot remove absent attribute member {member_key} from {subject_id}.{norm_key}"
                        ),
                    )));
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Validate a batch of events against `prior_state` in reducer order.
pub fn validate_event_batch_preconditions(
    actor: &str,
    action: CanonPolicyAction,
    batch: &[CanonEventV1],
    prior_state: &CanonStateViewV1,
) -> Result<(), CanonAppendError> {
    validate_event_batch_structure_and_conservation(actor, batch)?;

    let mut ordered_candidates: Vec<(usize, &CanonEventV1)> = batch.iter().enumerate().collect();
    ordered_candidates.sort_by(|(idx_a, ev_a), (idx_b, ev_b)| {
        let pos_key_a = ev_a.effective_position.map_or((0, 0, 0), |p| {
            (p.book_number, p.chapter_number, p.scene_order)
        });
        let pos_key_b = ev_b.effective_position.map_or((0, 0, 0), |p| {
            (p.book_number, p.chapter_number, p.scene_order)
        });
        let step_key_a = ev_a
            .within_scene_order
            .map_or((1u8, u32::MAX), |s| (0u8, s));
        let step_key_b = ev_b
            .within_scene_order
            .map_or((1u8, u32::MAX), |s| (0u8, s));
        pos_key_a
            .cmp(&pos_key_b)
            .then_with(|| step_key_a.cmp(&step_key_b))
            .then_with(|| ev_a.decision_seq.cmp(&ev_b.decision_seq))
            .then_with(|| ev_a.append_seq.cmp(&ev_b.append_seq))
            .then_with(|| idx_a.cmp(idx_b))
            .then_with(|| ev_a.event_id.cmp(&ev_b.event_id))
    });

    let mut working_state = prior_state.clone();
    for (idx, event) in ordered_candidates {
        validate_single_assertion_preconditions(actor, action, idx, event, &working_state)?;
        if !is_withdrawal_only_event(event) {
            apply_event_to_view(&mut working_state, event, event.effective_position).map_err(
                |mut err| {
                    err.event_index = Some(idx);
                    err
                },
            )?;
        }
    }

    Ok(())
}

fn reducer_event_cmp(
    depth_a: usize,
    pos_a: Option<CanonPositionV1>,
    ev_a: &CanonEventV1,
    depth_b: usize,
    pos_b: Option<CanonPositionV1>,
    ev_b: &CanonEventV1,
) -> std::cmp::Ordering {
    let pos_key_a = pos_a.map_or((0, 0, 0), |p| {
        (p.book_number, p.chapter_number, p.scene_order)
    });
    let pos_key_b = pos_b.map_or((0, 0, 0), |p| {
        (p.book_number, p.chapter_number, p.scene_order)
    });
    let step_key_a = ev_a
        .within_scene_order
        .map_or((1u8, u32::MAX), |s| (0u8, s));
    let step_key_b = ev_b
        .within_scene_order
        .map_or((1u8, u32::MAX), |s| (0u8, s));
    pos_key_a
        .cmp(&pos_key_b)
        .then_with(|| step_key_a.cmp(&step_key_b))
        .then_with(|| depth_b.cmp(&depth_a))
        .then_with(|| ev_a.decision_seq.cmp(&ev_b.decision_seq))
        .then_with(|| ev_a.append_seq.cmp(&ev_b.append_seq))
        .then_with(|| ev_a.event_id.cmp(&ev_b.event_id))
}

fn is_assertion_visible_at_position(
    depth: usize,
    effective_pos: Option<CanonPositionV1>,
    event: &CanonEventV1,
    effective_lineage_len: usize,
    masks_by_depth: &[(usize, i64, i64, bool, StreamCoverageMaskV1)],
    query_pos: Option<CanonPositionV1>,
    is_unbounded_query: bool,
) -> bool {
    let masked_by_descendant =
        masks_by_depth
            .iter()
            .any(|(mask_depth, mask_dec, mask_app, is_merged, mask)| {
                let mask_applies = *mask_depth < depth
                    || (*mask_depth == depth
                        && (event.decision_seq, event.append_seq) < (*mask_dec, *mask_app)
                        && (mask.tombstone
                            || *is_merged
                            || matches!(mask.target, CoverageMaskTargetV1::AncestorEvent { .. })));
                if !mask_applies || !mask_matches_event_target(mask, event) {
                    return false;
                }
                match &mask.target {
                    CoverageMaskTargetV1::AncestorEvent { .. } => true,
                    CoverageMaskTargetV1::Stream { .. }
                    | CoverageMaskTargetV1::LegacyWholeRecord { .. } => {
                        if is_delta_event(event) && !mask.tombstone {
                            // Delta streams: any ancestor delta inside `[start, end]` is masked.
                            mask.covers_position(effective_pos)
                        } else {
                            // State/collection streams (or tombstone mask):
                            // - Before `mask.start_position`, ancestor state remains visible.
                            // - During `[mask.start_position, mask.end_position]`, ancestor state at
                            //   or before `mask.end_position` is shadowed by the child stream.
                            // - After a bounded `mask.end_position`, inheritance of ancestor state resumes.
                            let cursor_in_mask = match query_pos {
                                Some(c_pos) => mask.covers_position(Some(c_pos)),
                                None if is_unbounded_query => mask.end_position.is_none(),
                                None => mask.covers_position(None),
                            };
                            if cursor_in_mask {
                                effective_pos.is_none_or(|ev_pos| {
                                    mask.end_position.is_none_or(|end| ev_pos <= end)
                                })
                            } else {
                                false
                            }
                        }
                    }
                }
            });
    if masked_by_descendant {
        return false;
    }

    // Also if this event is on a child branch (`depth`) that declared a bounded
    // coverage mask `[start, Some(end)]` for this stream, and the query cursor
    // is strictly after `end` (`cursor > end`), then the child's bounded override
    // interval has ended and ancestor inheritance has resumed!
    if !is_delta_event(event) {
        let expired_child_override =
            masks_by_depth
                .iter()
                .any(|(mask_depth, _mask_dec, _mask_app, is_merged, mask)| {
                    let same_override_layer = *mask_depth == depth
                        && (depth + 1 < effective_lineage_len
                            || (*is_merged && event.provenance == CanonProvenanceKind::Merged));
                    if !same_override_layer || !mask_matches_event_target(mask, event) {
                        return false;
                    }
                    let Some(end_pos) = mask.end_position else {
                        return false;
                    };
                    if !mask.covers_position(effective_pos) {
                        return false;
                    }
                    match query_pos {
                        Some(c_pos) => c_pos > end_pos,
                        None => is_unbounded_query,
                    }
                });
        if expired_child_override {
            return false;
        }
    }

    true
}

/// Validate a batch of candidate events against the full branch lineage history,
/// building the pre-event state in exact reducer order (including within-scene step
/// bounds, cursor-dependent coverage mask intervals, and intervening history across
/// multi-position batches).
pub fn validate_event_batch_in_history(
    branch_id: &str,
    lineage: &[String],
    existing_events: &[CanonEventV1],
    actor: &str,
    action: CanonPolicyAction,
    tier: CanonTier,
    candidate_events: &[CanonEventV1],
) -> Result<(), CanonAppendError> {
    validate_event_batch_structure_and_conservation(actor, candidate_events)?;

    let max_existing_decision_seq = existing_events
        .iter()
        .map(|e| e.decision_seq)
        .max()
        .unwrap_or(0);
    let max_existing_append_seq = existing_events
        .iter()
        .map(|e| e.append_seq)
        .max()
        .unwrap_or(0);

    let mut normalized_candidates: Vec<CanonEventV1> = Vec::with_capacity(candidate_events.len());
    for (idx, cand) in candidate_events.iter().enumerate() {
        let mut cloned = cand.clone();
        if cloned.decision_seq <= max_existing_decision_seq {
            cloned.decision_seq = max_existing_decision_seq + 1;
        }
        let min_append_seq = max_existing_append_seq + 1 + idx as i64;
        if cloned.append_seq < min_append_seq {
            cloned.append_seq = min_append_seq;
        }
        normalized_candidates.push(cloned);
    }

    let mut combined_events: Vec<CanonEventV1> =
        Vec::with_capacity(existing_events.len() + normalized_candidates.len());
    combined_events.extend_from_slice(existing_events);
    combined_events.extend(normalized_candidates.iter().cloned());

    let (effective_lineage, max_decision_seq, max_event_seq, active_lineage_events) =
        active_lineage_events_at_watermark(branch_id, lineage, &combined_events, tier, None);

    let mut suppressed_event_ids: Vec<(usize, i64, i64, String)> = Vec::new();
    let mut invalidated_revisions: Vec<(usize, i64, i64, Option<String>, String)> = Vec::new();
    let mut scene_visibility_by_id: BTreeMap<String, (usize, ScenePlacementVisibilityV1)> =
        BTreeMap::new();
    let mut masks_by_depth: Vec<(usize, i64, i64, bool, StreamCoverageMaskV1)> = Vec::new();

    let mut control_scan = active_lineage_events.clone();
    control_scan
        .sort_by_key(|(depth, e)| (std::cmp::Reverse(*depth), e.decision_seq, e.append_seq));

    for &(depth, event) in &control_scan {
        if let Some(withdrawn) = &event.withdraws_event_id {
            suppressed_event_ids.push((
                depth,
                event.decision_seq,
                event.append_seq,
                withdrawn.clone(),
            ));
        }
        if let Some(superseded) = &event.supersedes_event_id {
            suppressed_event_ids.push((
                depth,
                event.decision_seq,
                event.append_seq,
                superseded.clone(),
            ));
        }
        match &event.payload {
            CanonEventPayloadV1::SourceInvalidation { invalidation } => {
                for rev_id in &invalidation.invalidated_revision_ids {
                    invalidated_revisions.push((
                        depth,
                        event.decision_seq,
                        event.append_seq,
                        invalidation.scene_id.clone(),
                        rev_id.clone(),
                    ));
                }
                for ev_id in &invalidation.invalidated_event_ids {
                    suppressed_event_ids.push((
                        depth,
                        event.decision_seq,
                        event.append_seq,
                        ev_id.clone(),
                    ));
                }
            }
            CanonEventPayloadV1::SceneVisibility { visibility } => {
                scene_visibility_by_id
                    .insert(visibility.scene_id.clone(), (depth, visibility.clone()));
            }
            CanonEventPayloadV1::CoverageMask { mask } => {
                masks_by_depth.push((
                    depth,
                    event.decision_seq,
                    event.append_seq,
                    event.provenance == CanonProvenanceKind::Merged,
                    mask.clone(),
                ));
            }
            CanonEventPayloadV1::Assertion { .. }
            | CanonEventPayloadV1::WatermarkRestore { .. } => {}
        }
    }

    let mut unmasked_assertions: Vec<(usize, Option<CanonPositionV1>, &CanonEventV1)> = Vec::new();
    for (depth, event) in active_lineage_events {
        if !matches!(event.payload, CanonEventPayloadV1::Assertion { .. })
            || is_withdrawal_only_event(event)
        {
            continue;
        }
        if suppressed_event_ids
            .iter()
            .any(|(sup_depth, sup_dec, sup_app, id)| {
                (*sup_depth < depth
                    || (*sup_depth == depth
                        && (event.decision_seq, event.append_seq) < (*sup_dec, *sup_app)))
                    && event_matches_id_or_merged_source(event, id)
            })
        {
            continue;
        }
        if invalidated_revisions.iter().any(
            |(inv_depth, inv_dec, inv_app, scene_filter, rev_id)| {
                (*inv_depth < depth
                    || (*inv_depth == depth
                        && (event.decision_seq, event.append_seq) < (*inv_dec, *inv_app)))
                    && event_matches_revision_or_merged_source(event, rev_id)
                    && scene_filter
                        .as_ref()
                        .is_none_or(|s| event.scene_id.as_ref() == Some(s))
            },
        ) {
            continue;
        }

        let mut effective_pos = event.effective_position;
        if let Some(scene_id) = &event.scene_id
            && let Some((vis_depth, vis)) = scene_visibility_by_id.get(scene_id)
            && *vis_depth <= depth
        {
            if !vis.visible {
                continue;
            }
            if let Some(override_pos) = vis.position_override {
                effective_pos = Some(override_pos);
            }
        }

        unmasked_assertions.push((depth, effective_pos, event));
    }

    unmasked_assertions.sort_by(|(depth_a, pos_a, ev_a), (depth_b, pos_b, ev_b)| {
        reducer_event_cmp(*depth_a, *pos_a, ev_a, *depth_b, *pos_b, ev_b)
    });

    for (cand_idx, cand) in normalized_candidates.iter().enumerate() {
        if !matches!(cand.payload, CanonEventPayloadV1::Assertion { .. })
            || is_withdrawal_only_event(cand)
        {
            continue;
        }

        let cand_depth = effective_lineage
            .iter()
            .position(|b| b == &cand.branch_id)
            .unwrap_or(0);
        let mut cand_effective_pos = cand.effective_position;
        if let Some(scene_id) = &cand.scene_id
            && let Some((vis_depth, vis)) = scene_visibility_by_id.get(scene_id)
            && *vis_depth <= cand_depth
            && let Some(override_pos) = vis.position_override
        {
            cand_effective_pos = Some(override_pos);
        }

        let mut pre_cand_state = CanonStateViewV1 {
            branch_id: branch_id.to_string(),
            lineage: effective_lineage.clone(),
            cursor: CanonCursorV1 {
                position: cand_effective_pos,
                within_scene_order: cand.within_scene_order,
            },
            tier,
            decision_watermark: max_decision_seq,
            event_watermark: max_event_seq,
            reducer_version: CANON_REDUCER_VERSION,
            entities: BTreeMap::new(),
            relationships: BTreeMap::new(),
            quantities: BTreeMap::new(),
            knowledge: BTreeMap::new(),
            rules: BTreeMap::new(),
            timeline: ProjectedTimelineV1 {
                scene_clocks: BTreeMap::new(),
                latest_clock: None,
                source_event_ids: Vec::new(),
                reducer_version: CANON_REDUCER_VERSION,
            },
            open_threads: BTreeMap::new(),
            uncertain_orderings: Vec::new(),
            precondition_anomalies: Vec::new(),
            invalidated_scene_suffixes: Vec::new(),
            visible_event_ids: Vec::new(),
        };

        for &(depth, effective_pos, event) in &unmasked_assertions {
            if reducer_event_cmp(
                depth,
                effective_pos,
                event,
                cand_depth,
                cand_effective_pos,
                cand,
            ) != std::cmp::Ordering::Less
            {
                break;
            }
            if !is_assertion_visible_at_position(
                depth,
                effective_pos,
                event,
                effective_lineage.len(),
                &masks_by_depth,
                cand_effective_pos,
                false,
            ) {
                continue;
            }
            apply_event_to_view(&mut pre_cand_state, event, effective_pos)?;
        }

        validate_single_assertion_preconditions(actor, action, cand_idx, cand, &pre_cand_state)?;
    }

    Ok(())
}

/// Conflict reported by 3-way branch merge across common ancestor, target, and source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonMergeConflictV1 {
    pub kind: CanonMergeConflictKind,
    pub stream_description: String,
    pub target_event_ids: Vec<String>,
    pub source_event_ids: Vec<String>,
    pub detail: String,
}

/// Output of comparing common ancestor, target, and source canon event streams.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonMergePlanV1 {
    pub target_branch_id: String,
    pub source_branch_id: String,
    pub common_ancestor_branch_id: String,
    pub conflicts: Vec<CanonMergeConflictV1>,
    /// Source-branch events accepted for target-local append (with preserved source
    /// provenance, relationship contribution ledgers, and predecessor links).
    pub accepted_source_events: Vec<CanonEventV1>,
}

/// Pure 3-way merge analysis comparing `common_ancestor`, `target`, and `source`
/// branches. Detects divergent scalar sets, removal conflicts, and duplicate
/// transfers while preserving source provenance and never mutating source history.
#[allow(clippy::too_many_arguments)]
pub fn plan_canon_branch_merge(
    common_ancestor_branch_id: &str,
    ancestor_lineage: &[String],
    target_branch_id: &str,
    target_lineage: &[String],
    source_branch_id: &str,
    source_lineage: &[String],
    events: &[CanonEventV1],
    decision_watermark: Option<i64>,
) -> Result<CanonMergePlanV1, CanonAppendError> {
    let cursor = CanonCursorV1::unbounded();
    let ancestor_view = state_at(
        common_ancestor_branch_id,
        ancestor_lineage,
        events,
        &cursor,
        CanonTier::Ratified,
        decision_watermark,
    )?;
    let target_view = state_at(
        target_branch_id,
        target_lineage,
        events,
        &cursor,
        CanonTier::Ratified,
        decision_watermark,
    )?;
    let source_view = state_at(
        source_branch_id,
        source_lineage,
        events,
        &cursor,
        CanonTier::Ratified,
        decision_watermark,
    )?;

    let (_, _, _, ancestor_active_events) = active_lineage_events_at_watermark(
        common_ancestor_branch_id,
        ancestor_lineage,
        events,
        CanonTier::Ratified,
        decision_watermark,
    );
    let (_, _, _, target_active_events) = active_lineage_events_at_watermark(
        target_branch_id,
        target_lineage,
        events,
        CanonTier::Ratified,
        decision_watermark,
    );
    let (_, _, _, source_active_events) = active_lineage_events_at_watermark(
        source_branch_id,
        source_lineage,
        events,
        CanonTier::Ratified,
        decision_watermark,
    );

    let mut ancestor_represented_ids: BTreeSet<&str> = ancestor_view
        .visible_event_ids
        .iter()
        .map(String::as_str)
        .collect();
    for (_, ev) in &ancestor_active_events {
        ancestor_represented_ids.insert(ev.event_id.as_str());
        if let Some(mp) = &ev.merge_provenance {
            ancestor_represented_ids.insert(mp.source_event_id.as_str());
        }
    }

    let mut target_represented_ids: BTreeSet<&str> = ancestor_represented_ids.clone();
    for id in &target_view.visible_event_ids {
        target_represented_ids.insert(id.as_str());
    }
    for (_, ev) in &target_active_events {
        target_represented_ids.insert(ev.event_id.as_str());
        if let Some(mp) = &ev.merge_provenance {
            target_represented_ids.insert(mp.source_event_id.as_str());
        }
    }

    let is_represented_on_target = |ev: &CanonEventV1| -> bool {
        target_represented_ids.contains(ev.event_id.as_str())
            || ev
                .merge_provenance
                .as_ref()
                .is_some_and(|mp| target_represented_ids.contains(mp.source_event_id.as_str()))
    };

    let events_by_id: BTreeMap<&str, &CanonEventV1> =
        events.iter().map(|e| (e.event_id.as_str(), e)).collect();

    // Events visible on target/source that were added after the common ancestor.
    let target_only_events: Vec<&CanonEventV1> = target_view
        .visible_event_ids
        .iter()
        .filter(|id| !ancestor_represented_ids.contains(id.as_str()))
        .filter_map(|id| events_by_id.get(id.as_str()).copied())
        .collect();

    // Collect source-only control/removal events (SceneVisibility, SourceInvalidation,
    // CoverageMask, and withdrawals) active on `source_lineage` after `common_ancestor`
    // and not already represented on `target`.
    let mut source_control_scan = source_active_events.clone();
    source_control_scan
        .sort_by_key(|(depth, e)| (std::cmp::Reverse(*depth), e.decision_seq, e.append_seq));
    let mut latest_source_scene_vis: BTreeMap<&str, &CanonEventV1> = BTreeMap::new();
    let mut source_control_events: Vec<&CanonEventV1> = Vec::new();
    for &(_depth, ev) in &source_control_scan {
        if let CanonEventPayloadV1::SceneVisibility { visibility } = &ev.payload {
            latest_source_scene_vis.insert(visibility.scene_id.as_str(), ev);
        }
    }
    for &(_depth, ev) in &source_control_scan {
        if is_represented_on_target(ev) {
            continue;
        }
        let is_control_or_removal = match &ev.payload {
            CanonEventPayloadV1::SceneVisibility { visibility } => latest_source_scene_vis
                .get(visibility.scene_id.as_str())
                .is_some_and(|latest| latest.event_id == ev.event_id),
            CanonEventPayloadV1::SourceInvalidation { .. }
            | CanonEventPayloadV1::CoverageMask { .. } => true,
            CanonEventPayloadV1::Assertion { .. } => is_withdrawal_only_event(ev),
            CanonEventPayloadV1::WatermarkRestore { .. } => false,
        };
        if is_control_or_removal {
            source_control_events.push(ev);
        }
    }

    let source_assertion_events: Vec<&CanonEventV1> = source_view
        .visible_event_ids
        .iter()
        .filter_map(|id| events_by_id.get(id.as_str()).copied())
        .filter(|ev| !is_represented_on_target(ev))
        .collect();

    let mut source_only_events: Vec<&CanonEventV1> =
        Vec::with_capacity(source_control_events.len() + source_assertion_events.len());
    source_only_events.extend(source_control_events.iter().copied());
    source_only_events.extend(source_assertion_events.iter().copied());

    let mut conflicts: Vec<CanonMergeConflictV1> = Vec::new();
    let mut conflicting_source_event_ids: BTreeSet<String> = BTreeSet::new();

    // Helper: identify source control/removal event IDs that removed an ancestor event
    // or entity stream when `source` has no surviving positive event on that stream.
    let find_source_removal_ids = |entity_id: &str, ancestor_event_ids: &[String]| -> Vec<String> {
        let mut matched: Vec<String> = Vec::new();
        for &ctrl in &source_control_events {
            let matches_ancestor =
                ancestor_event_ids.iter().any(|anc_id| {
                    let Some(&anc_ev) = events_by_id.get(anc_id.as_str()) else {
                        return false;
                    };
                    if ctrl
                        .withdraws_event_id
                        .as_ref()
                        .is_some_and(|w| event_matches_id_or_merged_source(anc_ev, w))
                        || ctrl
                            .supersedes_event_id
                            .as_ref()
                            .is_some_and(|s| event_matches_id_or_merged_source(anc_ev, s))
                    {
                        return true;
                    }
                    match &ctrl.payload {
                        CanonEventPayloadV1::SceneVisibility { visibility } => {
                            !visibility.visible
                                && anc_ev.scene_id.as_deref() == Some(visibility.scene_id.as_str())
                        }
                        CanonEventPayloadV1::SourceInvalidation { invalidation } => {
                            invalidation
                                .invalidated_event_ids
                                .iter()
                                .any(|id| event_matches_id_or_merged_source(anc_ev, id))
                                || (invalidation.invalidated_revision_ids.iter().any(|rev| {
                                    event_matches_revision_or_merged_source(anc_ev, rev)
                                }) && invalidation
                                    .scene_id
                                    .as_ref()
                                    .is_none_or(|s| anc_ev.scene_id.as_ref() == Some(s)))
                        }
                        CanonEventPayloadV1::CoverageMask { mask } => {
                            mask_matches_event_target(mask, anc_ev)
                        }
                        CanonEventPayloadV1::Assertion { .. }
                        | CanonEventPayloadV1::WatermarkRestore { .. } => false,
                    }
                });
            if matches_ancestor || ctrl.subject_key() == entity_id {
                push_unique_id(&mut matched, &ctrl.event_id);
            }
        }
        matched
    };

    // 1. Check divergent scalar sets & removal conflicts across entities.
    let all_entity_ids: BTreeSet<&String> = ancestor_view
        .entities
        .keys()
        .chain(target_view.entities.keys())
        .chain(source_view.entities.keys())
        .collect();

    for entity_id in all_entity_ids {
        let anc_ent = ancestor_view.entities.get(entity_id);
        let tgt_ent = target_view.entities.get(entity_id);
        let src_ent = source_view.entities.get(entity_id);

        // LifeStatus scalar check
        let anc_life = anc_ent.and_then(|e| e.life_status.value);
        let tgt_life = tgt_ent.and_then(|e| e.life_status.value);
        let src_life = src_ent.and_then(|e| e.life_status.value);
        if tgt_life != anc_life && src_life != anc_life && tgt_life != src_life {
            let kind = if tgt_life.is_none() || src_life.is_none() {
                CanonMergeConflictKind::RemovalConflict
            } else {
                CanonMergeConflictKind::DivergentScalarSet
            };
            let anc_ids = anc_ent
                .map(|e| e.life_status.source_event_ids.clone())
                .unwrap_or_default();
            let tgt_ids = tgt_ent
                .map(|e| e.life_status.source_event_ids.clone())
                .unwrap_or_default();
            let mut src_ids = src_ent
                .map(|e| e.life_status.source_event_ids.clone())
                .unwrap_or_default();
            if src_ids.is_empty() {
                src_ids = find_source_removal_ids(entity_id, &anc_ids);
            }
            conflicting_source_event_ids.extend(src_ids.iter().cloned());
            conflicts.push(CanonMergeConflictV1 {
                kind,
                stream_description: format!("{entity_id}.life_status"),
                target_event_ids: tgt_ids,
                source_event_ids: src_ids,
                detail: format!(
                    "divergent life_status on {entity_id}: target={tgt_life:?}, source={src_life:?}"
                ),
            });
        }

        // Location scalar check
        let anc_loc = anc_ent.and_then(|e| e.location_id.value.as_ref());
        let tgt_loc = tgt_ent.and_then(|e| e.location_id.value.as_ref());
        let src_loc = src_ent.and_then(|e| e.location_id.value.as_ref());
        if tgt_loc != anc_loc && src_loc != anc_loc && tgt_loc != src_loc {
            let kind = if tgt_loc.is_none() || src_loc.is_none() {
                CanonMergeConflictKind::RemovalConflict
            } else {
                CanonMergeConflictKind::DivergentScalarSet
            };
            let anc_ids = anc_ent
                .map(|e| e.location_id.source_event_ids.clone())
                .unwrap_or_default();
            let tgt_ids = tgt_ent
                .map(|e| e.location_id.source_event_ids.clone())
                .unwrap_or_default();
            let mut src_ids = src_ent
                .map(|e| e.location_id.source_event_ids.clone())
                .unwrap_or_default();
            if src_ids.is_empty() {
                src_ids = find_source_removal_ids(entity_id, &anc_ids);
            }
            conflicting_source_event_ids.extend(src_ids.iter().cloned());
            conflicts.push(CanonMergeConflictV1 {
                kind,
                stream_description: format!("{entity_id}.location"),
                target_event_ids: tgt_ids,
                source_event_ids: src_ids,
                detail: format!(
                    "divergent location on {entity_id}: target={tgt_loc:?}, source={src_loc:?}"
                ),
            });
        }

        // Attribute scalar & set-member removal checks
        let all_attr_keys: BTreeSet<&String> = anc_ent
            .into_iter()
            .flat_map(|e| e.attributes.keys())
            .chain(tgt_ent.into_iter().flat_map(|e| e.attributes.keys()))
            .chain(src_ent.into_iter().flat_map(|e| e.attributes.keys()))
            .collect();
        for attr_key in all_attr_keys {
            let anc_attr = anc_ent.and_then(|e| e.attributes.get(attr_key));
            let tgt_attr = tgt_ent.and_then(|e| e.attributes.get(attr_key));
            let src_attr = src_ent.and_then(|e| e.attributes.get(attr_key));

            let anc_val = anc_attr.and_then(|a| a.scalar_value.as_ref());
            let tgt_val = tgt_attr.and_then(|a| a.scalar_value.as_ref());
            let src_val = src_attr.and_then(|a| a.scalar_value.as_ref());
            if tgt_val != anc_val && src_val != anc_val && tgt_val != src_val {
                let kind = if tgt_val.is_none() || src_val.is_none() {
                    CanonMergeConflictKind::RemovalConflict
                } else {
                    CanonMergeConflictKind::DivergentScalarSet
                };
                let anc_ids = anc_attr
                    .map(|a| a.source_event_ids.clone())
                    .unwrap_or_default();
                let tgt_ids = tgt_attr
                    .map(|a| a.source_event_ids.clone())
                    .unwrap_or_default();
                let mut src_ids = src_attr
                    .map(|a| a.source_event_ids.clone())
                    .unwrap_or_default();
                if src_ids.is_empty() {
                    src_ids = find_source_removal_ids(entity_id, &anc_ids);
                }
                conflicting_source_event_ids.extend(src_ids.iter().cloned());
                conflicts.push(CanonMergeConflictV1 {
                    kind,
                    stream_description: format!("{entity_id}.attribute.{attr_key}"),
                    target_event_ids: tgt_ids,
                    source_event_ids: src_ids,
                    detail: format!(
                        "divergent attribute {entity_id}.{attr_key}: target={tgt_val:?}, source={src_val:?}"
                    ),
                });
            }
        }

        // Injury removal / modification conflict check
        let all_injury_ids: BTreeSet<&String> = anc_ent
            .into_iter()
            .flat_map(|e| e.injuries.keys())
            .chain(tgt_ent.into_iter().flat_map(|e| e.injuries.keys()))
            .chain(src_ent.into_iter().flat_map(|e| e.injuries.keys()))
            .collect();
        for injury_id in all_injury_ids {
            let anc_inj = anc_ent.and_then(|e| e.injuries.get(injury_id));
            let tgt_inj = tgt_ent.and_then(|e| e.injuries.get(injury_id));
            let src_inj = src_ent.and_then(|e| e.injuries.get(injury_id));

            let tgt_changed = tgt_inj != anc_inj;
            let src_changed = src_inj != anc_inj;
            if tgt_changed && src_changed && tgt_inj != src_inj {
                let tgt_removed = tgt_inj.is_none_or(|i| i.status == InjuryStatus::Healed);
                let src_removed = src_inj.is_none_or(|i| i.status == InjuryStatus::Healed);
                if tgt_removed != src_removed
                    || tgt_inj.map(|i| i.severity) != src_inj.map(|i| i.severity)
                {
                    let anc_ids = anc_inj
                        .map(|i| i.source_event_ids.clone())
                        .unwrap_or_default();
                    let tgt_ids = tgt_inj
                        .map(|i| i.source_event_ids.clone())
                        .unwrap_or_default();
                    let mut src_ids = src_inj
                        .map(|i| i.source_event_ids.clone())
                        .unwrap_or_default();
                    if src_ids.is_empty() {
                        src_ids = find_source_removal_ids(entity_id, &anc_ids);
                    }
                    conflicting_source_event_ids.extend(src_ids.iter().cloned());
                    conflicts.push(CanonMergeConflictV1 {
                        kind: CanonMergeConflictKind::RemovalConflict,
                        stream_description: format!("{entity_id}.injury.{injury_id}"),
                        target_event_ids: tgt_ids,
                        source_event_ids: src_ids,
                        detail: format!(
                            "conflicting injury changes on {entity_id}.{injury_id} between target and source"
                        ),
                    });
                }
            }
        }
    }

    // 2. Check duplicate transfers (quantity transaction_id collisions or conflicting item transfers).
    let mut target_tx_events: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut target_item_transfers: BTreeMap<
        (String, String, CustodyKind),
        (String, Option<String>),
    > = BTreeMap::new();
    for ev in &target_only_events {
        if let CanonEventPayloadV1::Assertion { assertion, .. } = &ev.payload {
            match assertion {
                ClaimAssertion::Quantity {
                    transaction_id: Some(tx_id),
                    ..
                } => {
                    target_tx_events
                        .entry(tx_id.clone())
                        .or_default()
                        .push((ev.event_id.clone(), ev.semantic_fingerprint.clone()));
                }
                ClaimAssertion::Possession {
                    operation: PossessionOperation::Transfer | PossessionOperation::Lose,
                    item,
                    counterparty,
                    custody,
                } => {
                    target_item_transfers.insert(
                        (ev.subject_key(), entity_key(item), *custody),
                        (ev.event_id.clone(), counterparty.as_ref().map(entity_key)),
                    );
                }
                _ => {}
            }
        }
    }

    for ev in &source_only_events {
        if let CanonEventPayloadV1::Assertion { assertion, .. } = &ev.payload {
            match assertion {
                ClaimAssertion::Quantity {
                    transaction_id: Some(tx_id),
                    ..
                } => {
                    if let Some(tgt_legs) = target_tx_events.get(tx_id) {
                        conflicting_source_event_ids.insert(ev.event_id.clone());
                        conflicts.push(CanonMergeConflictV1 {
                            kind: CanonMergeConflictKind::DuplicateTransfer,
                            stream_description: format!("quantity.transaction.{tx_id}"),
                            target_event_ids: tgt_legs.iter().map(|(id, _)| id.clone()).collect(),
                            source_event_ids: vec![ev.event_id.clone()],
                            detail: format!(
                                "duplicate quantity transfer transaction_id {tx_id} across target and source"
                            ),
                        });
                    }
                }
                ClaimAssertion::Possession {
                    operation: PossessionOperation::Transfer | PossessionOperation::Lose,
                    item,
                    counterparty,
                    custody,
                } => {
                    let key = (ev.subject_key(), entity_key(item), *custody);
                    if let Some((tgt_ev_id, tgt_other)) = target_item_transfers.get(&key) {
                        let src_other = counterparty.as_ref().map(entity_key);
                        if tgt_other != &src_other {
                            conflicting_source_event_ids.insert(ev.event_id.clone());
                            conflicts.push(CanonMergeConflictV1 {
                                kind: CanonMergeConflictKind::DuplicateTransfer,
                                stream_description: format!(
                                    "possession.{}.{}",
                                    key.0, key.1
                                ),
                                target_event_ids: vec![tgt_ev_id.clone()],
                                source_event_ids: vec![ev.event_id.clone()],
                                detail: format!(
                                    "conflicting transfer/loss of item {} from {} on target and source",
                                    key.1, key.0
                                ),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let accepted_source_events = source_only_events
        .into_iter()
        .filter(|ev| !conflicting_source_event_ids.contains(&ev.event_id))
        .map(|ev| {
            let mut cloned = ev.clone();
            if let CanonEventPayloadV1::SourceInvalidation { invalidation } = &mut cloned.payload
                && !invalidation.invalidated_revision_ids.is_empty()
            {
                for candidate in events {
                    if candidate.event_id == cloned.event_id {
                        continue;
                    }
                    let matches_rev = invalidation
                        .invalidated_revision_ids
                        .iter()
                        .any(|rev| event_matches_revision_or_merged_source(candidate, rev));
                    let matches_scene = invalidation
                        .scene_id
                        .as_ref()
                        .is_none_or(|s| candidate.scene_id.as_ref() == Some(s));
                    if matches_rev && matches_scene {
                        push_unique_id(
                            &mut invalidation.invalidated_event_ids,
                            &candidate.event_id,
                        );
                        if let Some(mp) = &candidate.merge_provenance {
                            push_unique_id(
                                &mut invalidation.invalidated_event_ids,
                                &mp.source_event_id,
                            );
                        }
                    }
                }
            }
            cloned
        })
        .collect();

    Ok(CanonMergePlanV1 {
        target_branch_id: target_branch_id.to_string(),
        source_branch_id: source_branch_id.to_string(),
        common_ancestor_branch_id: common_ancestor_branch_id.to_string(),
        conflicts,
        accepted_source_events,
    })
}
