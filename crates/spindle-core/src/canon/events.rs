//! Append-only canon event, claim, decision, source-map, and savepoint contracts.
//!
//! Core owns only pure DTOs, vocabularies, exact decimal helpers, semantic
//! fingerprints, and deterministic precondition checks. SQLite persistence and
//! service transactions live in `spindle-adapters`.

use super::comparison::{key_text, normalized_assertion, normalized_decimal};
use super::validation::decimal as validate_decimal_syntax;
use super::*;
use crate::models::{StoryClock, StoryPlacement};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const CANON_EVENT_SCHEMA_VERSION: u32 = 1;
pub const CANON_REDUCER_VERSION: u32 = 1;
pub const CANON_POLICY_VERSION_V1: &str = "canon-policy-v1";

vocabulary!(CanonTier {
    Ratified,
    Provisional
});

vocabulary!(CanonClaimStatus {
    Provisional,
    Ratified,
    Rejected,
    Withdrawn,
    SourceInvalidated
});

vocabulary!(CanonPolicyAction {
    StageProvisional,
    Ratify,
    Reject,
    Withdraw,
    Supersede,
    InvalidateSource,
    BranchMask,
    SceneVisibility,
    RestoreWatermark,
    MergeAccept
});

vocabulary!(CanonProvenanceKind {
    Extracted,
    DrafterPackage,
    ManualAuthor,
    LegacyUnverified,
    Merged,
    SystemRestoration
});

vocabulary!(CanonEventFamily {
    LifeAction,
    LocationClock,
    Possession,
    Injury,
    Quantity,
    KnowledgeDisclosure,
    AttributeWorldState,
    Relationship,
    Promise,
    RuleTemporal,
    StreamControl
});

vocabulary!(CanonSourceMapStatus {
    Mapped,
    Quarantined,
    DuplicateSkipped
});

vocabulary!(CanonProjectionKind {
    EntityState,
    Relationship,
    Quantity,
    Knowledge,
    RuleConstraint,
    TimelineClock,
    OpenThread
});

vocabulary!(InjuryStatus { Active, Healed });

vocabulary!(KnowledgeStatus {
    Known,
    Forgotten,
    Unknown
});

vocabulary!(PromiseThreadStatus {
    Planted,
    Reinforced,
    PaidOff,
    Abandoned,
    Unknown
});

vocabulary!(CanonMergeConflictKind {
    DivergentScalarSet,
    RemovalConflict,
    DuplicateTransfer
});

vocabulary!(CanonAppendErrorCode {
    Identity,
    Schema,
    PolicyGuard,
    Precondition,
    Conservation,
    UnitMismatch,
    UnresolvedEntity,
    DuplicateInBatch,
    InvalidMask,
    InvalidCursor
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonAppendError {
    pub code: CanonAppendErrorCode,
    pub event_index: Option<usize>,
    pub field: String,
    pub detail: String,
}

impl std::fmt::Display for CanonAppendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {} (event {:?}): {}",
            self.code, self.field, self.event_index, self.detail
        )
    }
}

impl std::error::Error for CanonAppendError {}

pub fn append_error(
    code: CanonAppendErrorCode,
    field: impl Into<String>,
    detail: impl Into<String>,
) -> CanonAppendError {
    CanonAppendError {
        code,
        event_index: None,
        field: field.into(),
        detail: detail.into(),
    }
}

/// Exact base-10 decimal value using checked `i128` scaled arithmetic (up to 18
/// fractional places) so quantity balances and transfer conservation never use
/// lossy binary floating-point operations.
#[derive(Debug, Clone, Copy, Eq)]
pub struct ExactDecimal {
    mantissa: i128,
    scale: u32,
}

impl ExactDecimal {
    pub const MAX_SCALE: u32 = 18;

    pub fn zero() -> Self {
        Self {
            mantissa: 0,
            scale: 0,
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CanonAppendError> {
        let trimmed = raw.trim();
        validate_decimal_syntax(trimmed).map_err(|_| {
            append_error(
                CanonAppendErrorCode::Schema,
                "decimal",
                "invalid decimal syntax",
            )
        })?;
        let canonical = normalized_decimal(trimmed);
        let negative = canonical.starts_with('-');
        let unsigned = canonical.trim_start_matches('-');
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        let scale = u32::try_from(fraction.len()).map_err(|_| {
            append_error(
                CanonAppendErrorCode::Schema,
                "decimal",
                "decimal scale overflow",
            )
        })?;
        if scale > Self::MAX_SCALE {
            return Err(append_error(
                CanonAppendErrorCode::Schema,
                "decimal",
                "decimal exceeds maximum supported fractional scale",
            ));
        }
        let combined = format!("{whole}{fraction}");
        let mut mantissa: i128 = combined.parse().map_err(|_| {
            append_error(
                CanonAppendErrorCode::Schema,
                "decimal",
                "decimal mantissa overflow",
            )
        })?;
        if negative && mantissa != 0 {
            mantissa = -mantissa;
        }
        Ok(Self { mantissa, scale }.normalized())
    }

    pub fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    pub fn is_negative(&self) -> bool {
        self.mantissa < 0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, CanonAppendError> {
        let (lhs, rhs, scale) = Self::align(self, other)?;
        let sum = lhs.checked_add(rhs).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal addition overflow",
            )
        })?;
        Ok(Self {
            mantissa: sum,
            scale,
        }
        .normalized())
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, CanonAppendError> {
        let (lhs, rhs, scale) = Self::align(self, other)?;
        let diff = lhs.checked_sub(rhs).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal subtraction overflow",
            )
        })?;
        Ok(Self {
            mantissa: diff,
            scale,
        }
        .normalized())
    }

    pub fn to_canonical_string(self) -> String {
        let norm = self.normalized();
        if norm.mantissa == 0 {
            return "0".into();
        }
        let negative = norm.mantissa < 0;
        let abs = norm.mantissa.unsigned_abs();
        if norm.scale == 0 {
            return if negative {
                format!("-{abs}")
            } else {
                abs.to_string()
            };
        }
        let factor = 10u128.pow(norm.scale);
        let whole = abs / factor;
        let rem = abs % factor;
        let frac = format!("{:0width$}", rem, width = norm.scale as usize);
        let frac = frac.trim_end_matches('0');
        let sign = if negative { "-" } else { "" };
        if frac.is_empty() {
            format!("{sign}{whole}")
        } else {
            format!("{sign}{whole}.{frac}")
        }
    }

    fn normalized(mut self) -> Self {
        if self.mantissa == 0 {
            return Self::zero();
        }
        while self.scale > 0 && self.mantissa % 10 == 0 {
            self.mantissa /= 10;
            self.scale -= 1;
        }
        self
    }

    fn align(a: Self, b: Self) -> Result<(i128, i128, u32), CanonAppendError> {
        let target_scale = a.scale.max(b.scale);
        if target_scale > Self::MAX_SCALE {
            return Err(append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal scale alignment overflow",
            ));
        }
        let scale_a = 10i128.checked_pow(target_scale - a.scale).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal power overflow",
            )
        })?;
        let scale_b = 10i128.checked_pow(target_scale - b.scale).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal power overflow",
            )
        })?;
        let lhs = a.mantissa.checked_mul(scale_a).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal align overflow",
            )
        })?;
        let rhs = b.mantissa.checked_mul(scale_b).ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Conservation,
                "decimal",
                "decimal align overflow",
            )
        })?;
        Ok((lhs, rhs, target_scale))
    }
}

impl PartialEq for ExactDecimal {
    fn eq(&self, other: &Self) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.mantissa == b.mantissa && a.scale == b.scale
    }
}

impl PartialOrd for ExactDecimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExactDecimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if let Ok((lhs, rhs, _)) = Self::align(*self, *other) {
            lhs.cmp(&rhs)
        } else {
            self.to_canonical_string().cmp(&other.to_canonical_string())
        }
    }
}

/// Validated narrative story position `(book_number, chapter_number, scene_order)`.
/// Unknown effective placement is represented as `Option<CanonPositionV1> = None`
/// (baseline of unknown effective time, never scene zero).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct CanonPositionV1 {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

impl CanonPositionV1 {
    pub fn new(
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Self, CanonAppendError> {
        if book_number < 1 || chapter_number < 1 || scene_order < 1 {
            return Err(append_error(
                CanonAppendErrorCode::Identity,
                "position",
                "narrative position requires book_number >= 1, chapter_number >= 1, scene_order >= 1",
            ));
        }
        Ok(Self {
            book_number,
            chapter_number,
            scene_order,
        })
    }

    pub fn from_placement(placement: &StoryPlacement) -> Result<Self, CanonAppendError> {
        let scene_order = placement.scene_order.ok_or_else(|| {
            append_error(
                CanonAppendErrorCode::Identity,
                "position.scene_order",
                "scene_order is required for a concrete narrative position",
            )
        })?;
        Self::new(placement.book_number, placement.chapter_number, scene_order)
    }

    pub fn to_placement(self) -> StoryPlacement {
        StoryPlacement {
            book_number: self.book_number,
            chapter_number: self.chapter_number,
            scene_order: Some(self.scene_order),
            note: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonCursorV1 {
    /// `None` folds all visible events through the end of the branch.
    pub position: Option<CanonPositionV1>,
    /// Optional within-scene step cursor when `position` is `Some`.
    pub within_scene_order: Option<u32>,
}

impl CanonCursorV1 {
    pub fn unbounded() -> Self {
        Self {
            position: None,
            within_scene_order: None,
        }
    }

    pub fn at_scene(position: CanonPositionV1) -> Self {
        Self {
            position: Some(position),
            within_scene_order: None,
        }
    }

    pub fn at_within_scene(position: CanonPositionV1, within_scene_order: u32) -> Self {
        Self {
            position: Some(position),
            within_scene_order: Some(within_scene_order),
        }
    }

    pub fn validate(&self) -> Result<(), CanonAppendError> {
        if self.position.is_none() && self.within_scene_order.is_some() {
            return Err(append_error(
                CanonAppendErrorCode::InvalidCursor,
                "cursor.within_scene_order",
                "within_scene_order cursor requires a concrete scene position",
            ));
        }
        if let Some(pos) = self.position {
            CanonPositionV1::new(pos.book_number, pos.chapter_number, pos.scene_order)?;
        }
        Ok(())
    }
}

/// Semantic stream key used for child-over-parent visibility, collection member
/// identity, and branch merge comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "stream_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticStreamKeyV1 {
    LifeStatus {
        subject_id: String,
    },
    Action {
        subject_id: String,
        verb: String,
    },
    Location {
        subject_id: String,
    },
    SceneClock {
        position: CanonPositionV1,
    },
    PossessionItem {
        subject_id: String,
        item_id: String,
        custody: CustodyKind,
    },
    InjuryMember {
        subject_id: String,
        injury_id: String,
    },
    QuantityMeasure {
        subject_id: String,
        measure: String,
        unit: String,
    },
    KnowledgeProposition {
        subject_id: String,
        proposition_id: String,
    },
    AttributeScalar {
        subject_id: String,
        key: String,
    },
    AttributeMember {
        subject_id: String,
        key: String,
        member: String,
    },
    RelationshipPair {
        subject_id: String,
        other_id: String,
    },
    PromiseThread {
        thread_id: String,
    },
    RuleConstraint {
        rule_id: String,
    },
}

/// Target scope of a branch-local coverage mask.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mask_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoverageMaskTargetV1 {
    /// Masks a single semantic stream (or collection member) over `[start_position, end_position]`.
    Stream { stream: SemanticStreamKeyV1 },
    /// Explicitly masks one inherited ancestor event (e.g. a quantity or relationship
    /// delta contribution) on the child branch without inventing a compensating debit
    /// or mutating the parent branch.
    AncestorEvent { event_id: String },
    /// Legacy natural-key whole-record coverage mask (e.g. `character_state`,
    /// `canonical_fact`, `world_state`). Preserves whole-record override semantics
    /// until a project explicitly upgrades to predicate-level masks.
    LegacyWholeRecord {
        record_kind: String,
        subject_id: String,
        natural_key: String,
    },
}

/// Explicit child-branch coverage interval. Before `start_position`, ancestor state
/// remains visible; during `[start_position, end_position]` it shadows the ancestor
/// stream; after `end_position` inheritance resumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamCoverageMaskV1 {
    pub target: CoverageMaskTargetV1,
    pub start_position: Option<CanonPositionV1>,
    pub end_position: Option<CanonPositionV1>,
    pub tombstone: bool,
}

impl StreamCoverageMaskV1 {
    pub fn validate(&self) -> Result<(), CanonAppendError> {
        if let Some(pos) = self.start_position {
            CanonPositionV1::new(pos.book_number, pos.chapter_number, pos.scene_order)?;
        }
        if let Some(pos) = self.end_position {
            CanonPositionV1::new(pos.book_number, pos.chapter_number, pos.scene_order)?;
        }
        if let (Some(start), Some(end)) = (self.start_position, self.end_position)
            && start > end
        {
            return Err(append_error(
                CanonAppendErrorCode::InvalidMask,
                "coverage_mask",
                "coverage mask start_position cannot exceed end_position",
            ));
        }
        match &self.target {
            CoverageMaskTargetV1::AncestorEvent { event_id } => {
                if event_id.trim().is_empty() {
                    return Err(append_error(
                        CanonAppendErrorCode::InvalidMask,
                        "coverage_mask.event_id",
                        "ancestor event mask requires nonempty event_id",
                    ));
                }
            }
            CoverageMaskTargetV1::LegacyWholeRecord {
                record_kind,
                subject_id,
                natural_key,
            } => {
                if record_kind.trim().is_empty()
                    || subject_id.trim().is_empty()
                    || natural_key.trim().is_empty()
                {
                    return Err(append_error(
                        CanonAppendErrorCode::InvalidMask,
                        "coverage_mask.legacy_whole_record",
                        "legacy whole-record mask requires nonempty record_kind, subject_id, and natural_key",
                    ));
                }
            }
            CoverageMaskTargetV1::Stream { .. } => {}
        }
        Ok(())
    }

    pub fn covers_position(&self, position: Option<CanonPositionV1>) -> bool {
        match position {
            None => self.start_position.is_none(),
            Some(pos) => {
                let after_start = self.start_position.is_none_or(|start| pos >= start);
                let before_end = self.end_position.is_none_or(|end| pos <= end);
                after_start && before_end
            }
        }
    }
}

/// Optional legacy whole-record scope tag attached to backfilled or compatibility
/// events so legacy whole-record masks match their exact natural key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LegacyRecordScopeV1 {
    pub record_kind: String,
    pub natural_key: String,
}

/// Provenance recorded when a branch merge accepts a source-branch event onto a target branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeEventProvenanceV1 {
    pub source_branch_id: String,
    pub source_event_id: String,
    pub source_decision_id: String,
    pub common_ancestor_branch_id: Option<String>,
    #[serde(default)]
    pub source_revision_id: Option<String>,
}

/// Versioned scene-placement and visibility mapping for move/delete/restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenePlacementVisibilityV1 {
    pub scene_id: String,
    pub visible: bool,
    pub position_override: Option<CanonPositionV1>,
    pub invalidated_from: CanonPositionV1,
}

/// Source-invalidation record appended when a scene is revised or remined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceInvalidationV1 {
    pub scene_id: Option<String>,
    pub invalidated_revision_ids: Vec<String>,
    pub invalidated_event_ids: Vec<String>,
    pub replacement_revision_id: Option<String>,
}

/// Savepoint restoration watermark record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatermarkRestoreV1 {
    pub save_point_id: Option<String>,
    pub target_decision_seq: i64,
    pub target_event_seq: i64,
}

/// Discriminated payload stored on a `CanonEventV1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonEventPayloadV1 {
    Assertion {
        assertion: ClaimAssertion,
        modality: ClaimModality,
        temporal_scope: TemporalScope,
        speaker: Option<ClaimEntity>,
        experiencer: Option<ClaimEntity>,
    },
    CoverageMask {
        mask: StreamCoverageMaskV1,
    },
    SourceInvalidation {
        invalidation: SourceInvalidationV1,
    },
    SceneVisibility {
        visibility: ScenePlacementVisibilityV1,
    },
    WatermarkRestore {
        restore: WatermarkRestoreV1,
    },
}

impl CanonEventPayloadV1 {
    pub fn family(&self) -> CanonEventFamily {
        match self {
            Self::Assertion { assertion, .. } => match assertion.family() {
                ClaimFamily::LifeAction => CanonEventFamily::LifeAction,
                ClaimFamily::LocationClock => CanonEventFamily::LocationClock,
                ClaimFamily::Possession => CanonEventFamily::Possession,
                ClaimFamily::Injury => CanonEventFamily::Injury,
                ClaimFamily::Quantity => CanonEventFamily::Quantity,
                ClaimFamily::KnowledgeDisclosure => CanonEventFamily::KnowledgeDisclosure,
                ClaimFamily::AttributeWorldState => CanonEventFamily::AttributeWorldState,
                ClaimFamily::Relationship => CanonEventFamily::Relationship,
                ClaimFamily::Promise => CanonEventFamily::Promise,
                ClaimFamily::RuleTemporal => CanonEventFamily::RuleTemporal,
            },
            Self::CoverageMask { .. }
            | Self::SourceInvalidation { .. }
            | Self::SceneVisibility { .. }
            | Self::WatermarkRestore { .. } => CanonEventFamily::StreamControl,
        }
    }

    pub fn predicate_and_operation(&self) -> (&'static str, &'static str) {
        match self {
            Self::Assertion { assertion, .. } => match assertion {
                ClaimAssertion::LifeStatus { .. } => ("life_status", "set"),
                ClaimAssertion::Action { .. } => ("action", "occurrence"),
                ClaimAssertion::LocatedAt { .. } => ("located_at", "set"),
                ClaimAssertion::Move { .. } => ("move", "transition"),
                ClaimAssertion::SceneClock { .. } => ("scene_clock", "set"),
                ClaimAssertion::Possession { operation, .. } => (
                    "possession",
                    match operation {
                        PossessionOperation::Acquire => "acquire",
                        PossessionOperation::Transfer => "transfer",
                        PossessionOperation::Lose => "lose",
                    },
                ),
                ClaimAssertion::Injury { operation, .. } => (
                    "injury",
                    match operation {
                        InjuryOperation::Add => "add",
                        InjuryOperation::Heal => "heal",
                    },
                ),
                ClaimAssertion::Quantity { operation, .. } => (
                    "quantity",
                    match operation {
                        QuantityOperation::Set => "set",
                        QuantityOperation::Delta => "delta",
                    },
                ),
                ClaimAssertion::Knowledge { operation, .. } => (
                    "knowledge",
                    match operation {
                        KnowledgeOperation::Learn => "learn",
                        KnowledgeOperation::Forget => "forget",
                        KnowledgeOperation::Use => "use",
                        KnowledgeOperation::Disclose => "disclose",
                    },
                ),
                ClaimAssertion::Attribute { operation, .. } => (
                    "attribute",
                    match operation {
                        AttributeOperation::Set => "set",
                        AttributeOperation::Add => "add",
                        AttributeOperation::Remove => "remove",
                    },
                ),
                ClaimAssertion::Relationship { operation, .. } => (
                    "relationship",
                    match operation {
                        RelationshipOperation::Delta => "delta",
                        RelationshipOperation::Observation => "observation",
                    },
                ),
                ClaimAssertion::Promise { operation, .. } => (
                    "promise",
                    match operation {
                        PromiseOperation::Plant => "plant",
                        PromiseOperation::Reinforce => "reinforce",
                        PromiseOperation::Payoff => "payoff",
                        PromiseOperation::Abandon => "abandon",
                    },
                ),
                ClaimAssertion::Rule { operation, .. } => (
                    "rule",
                    match operation {
                        RuleOperation::Define => "define",
                        RuleOperation::Use => "use",
                        RuleOperation::Anchor => "anchor",
                        RuleOperation::Intervene => "intervene",
                    },
                ),
            },
            Self::CoverageMask { .. } => ("coverage_mask", "mask"),
            Self::SourceInvalidation { .. } => ("source_invalidation", "invalidate"),
            Self::SceneVisibility { .. } => ("scene_visibility", "visibility"),
            Self::WatermarkRestore { .. } => ("watermark_restore", "restore"),
        }
    }
}

/// Stored claim record in `canon_claim`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonClaimRecordV1 {
    pub claim_record_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: Option<String>,
    pub revision_id: String,
    pub source: ClaimSource,
    pub semantic_fingerprint: String,
    pub status: CanonClaimStatus,
    pub tier: CanonTier,
    pub provenance: CanonProvenanceKind,
    pub claim: ClaimV1,
    pub created_at_micros: i64,
}

/// Immutable append-only canon event in `canon_event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonEventV1 {
    pub event_id: String,
    pub schema_version: u32,
    pub project_id: String,
    pub branch_id: String,
    pub decision_id: String,
    pub decision_seq: i64,
    pub append_seq: i64,
    pub tier: CanonTier,
    pub policy_action: CanonPolicyAction,
    pub subject: ClaimEntity,
    pub predicate_family: CanonEventFamily,
    pub predicate: String,
    pub operation: String,
    pub payload: CanonEventPayloadV1,
    /// `None` represents a baseline with unknown effective narrative position,
    /// never scene zero.
    pub effective_position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub in_world_time: Option<StoryClock>,
    pub scene_id: Option<String>,
    pub source_revision_id: String,
    pub source_claim_ids: Vec<String>,
    pub semantic_fingerprint: String,
    pub idempotency_key: String,
    pub evidence_sha256: String,
    pub provenance: CanonProvenanceKind,
    pub supersedes_event_id: Option<String>,
    pub withdraws_event_id: Option<String>,
    pub legacy_record_scope: Option<LegacyRecordScopeV1>,
    pub merge_provenance: Option<MergeEventProvenanceV1>,
    pub created_at_micros: i64,
}

impl CanonEventV1 {
    pub fn resolved_subject_id(&self) -> Option<&str> {
        match &self.subject {
            ClaimEntity::Resolved { id } => Some(id.as_str()),
            ClaimEntity::Unresolved { .. } => None,
        }
    }

    pub fn subject_key(&self) -> String {
        match &self.subject {
            ClaimEntity::Resolved { id } => id.clone(),
            ClaimEntity::Unresolved { label } => format!("unresolved:{label}"),
        }
    }

    pub fn semantic_stream_key(&self) -> Option<SemanticStreamKeyV1> {
        let subject_id = self.subject_key();
        match &self.payload {
            CanonEventPayloadV1::Assertion { assertion, .. } => match assertion {
                ClaimAssertion::LifeStatus { .. } => {
                    Some(SemanticStreamKeyV1::LifeStatus { subject_id })
                }
                ClaimAssertion::Action { verb, .. } => Some(SemanticStreamKeyV1::Action {
                    subject_id,
                    verb: key_text(verb),
                }),
                ClaimAssertion::LocatedAt { .. } | ClaimAssertion::Move { .. } => {
                    Some(SemanticStreamKeyV1::Location { subject_id })
                }
                ClaimAssertion::SceneClock { .. } => self
                    .effective_position
                    .map(|position| SemanticStreamKeyV1::SceneClock { position }),
                ClaimAssertion::Possession { item, custody, .. } => {
                    Some(SemanticStreamKeyV1::PossessionItem {
                        subject_id,
                        item_id: entity_key(item),
                        custody: *custody,
                    })
                }
                ClaimAssertion::Injury { injury_id, .. } => {
                    Some(SemanticStreamKeyV1::InjuryMember {
                        subject_id,
                        injury_id: injury_id.trim().to_string(),
                    })
                }
                ClaimAssertion::Quantity { measure, unit, .. } => {
                    Some(SemanticStreamKeyV1::QuantityMeasure {
                        subject_id,
                        measure: key_text(measure),
                        unit: unit.trim().to_string(),
                    })
                }
                ClaimAssertion::Knowledge { proposition, .. } => {
                    Some(SemanticStreamKeyV1::KnowledgeProposition {
                        subject_id,
                        proposition_id: entity_key(proposition),
                    })
                }
                ClaimAssertion::Attribute {
                    operation,
                    key,
                    value,
                    ..
                } => {
                    let norm_key = key_text(key);
                    match operation {
                        AttributeOperation::Set => Some(SemanticStreamKeyV1::AttributeScalar {
                            subject_id,
                            key: norm_key,
                        }),
                        AttributeOperation::Add | AttributeOperation::Remove => {
                            Some(SemanticStreamKeyV1::AttributeMember {
                                subject_id,
                                key: norm_key,
                                member: scalar_member_key(value),
                            })
                        }
                    }
                }
                ClaimAssertion::Relationship { other, .. } => {
                    Some(SemanticStreamKeyV1::RelationshipPair {
                        subject_id,
                        other_id: entity_key(other),
                    })
                }
                ClaimAssertion::Promise { thread, .. } => {
                    Some(SemanticStreamKeyV1::PromiseThread {
                        thread_id: entity_key(thread),
                    })
                }
                ClaimAssertion::Rule { rule, .. } => Some(SemanticStreamKeyV1::RuleConstraint {
                    rule_id: entity_key(rule),
                }),
            },
            CanonEventPayloadV1::CoverageMask { .. }
            | CanonEventPayloadV1::SourceInvalidation { .. }
            | CanonEventPayloadV1::SceneVisibility { .. }
            | CanonEventPayloadV1::WatermarkRestore { .. } => None,
        }
    }
}

/// Audit record of a ratification, staging, invalidation, mask, restore, or merge decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonDecisionV1 {
    pub decision_id: String,
    pub decision_seq: i64,
    pub project_id: String,
    pub branch_id: String,
    pub action: CanonPolicyAction,
    pub actor: String,
    pub policy_version: String,
    pub source_revision_id: String,
    pub semantic_fingerprint: String,
    pub idempotency_key: String,
    pub projection_watermark: i64,
    pub previous_decision_id: Option<String>,
    pub target_scene_id: Option<String>,
    pub confidence_inputs: Vec<f64>,
    pub precondition_summary: String,
    pub reason: String,
    pub event_ids: Vec<String>,
    pub created_at_micros: i64,
}

/// Backfill and legacy source mapping row in `canon_source_map`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonSourceMapEntryV1 {
    pub project_id: String,
    pub branch_id: String,
    pub source_table: String,
    pub legacy_primary_key: String,
    pub migration_version: u32,
    pub status: CanonSourceMapStatus,
    pub provenance: CanonProvenanceKind,
    pub claim_ids: Vec<String>,
    pub event_ids: Vec<String>,
    pub decision_id: Option<String>,
    pub quarantine_reason: Option<String>,
    pub raw_snapshot_json: String,
    pub created_at_micros: i64,
    pub updated_at_micros: i64,
}

/// Savepoint checkpoint capturing reducer version, lineage, and decision/event watermarks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonSavepointCheckpointV1 {
    pub checkpoint_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub save_point_id: Option<String>,
    pub tier: CanonTier,
    pub lineage: Vec<String>,
    pub decision_watermark: i64,
    pub event_watermark: i64,
    pub branch_decision_watermarks: BTreeMap<String, i64>,
    pub reducer_version: u32,
    pub schema_version: u32,
    pub created_at_micros: i64,
}

pub fn entity_key(entity: &ClaimEntity) -> String {
    match entity {
        ClaimEntity::Resolved { id } => id.clone(),
        ClaimEntity::Unresolved { label } => format!("unresolved:{}", label.trim()),
    }
}

pub fn scalar_member_key(scalar: &ClaimScalar) -> String {
    match scalar {
        ClaimScalar::Text { value } => format!("text:{}", value.trim()),
        ClaimScalar::Boolean { value } => format!("bool:{value}"),
        ClaimScalar::Decimal { value, unit } => format!(
            "decimal:{}:{}",
            normalized_decimal(value),
            unit.as_deref().unwrap_or("").trim()
        ),
    }
}

pub fn evidence_sha256(evidence: &[ClaimEvidence]) -> String {
    let bytes = serde_json::to_vec(evidence).expect("claim evidence serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Deterministic semantic fingerprint for a claim, independent of transient
/// `claim_id`, `extraction_run_id`, or `model_call_id` so retries of the same
/// revision deduplicate while distinct occurrences at the same position remain distinct.
pub fn semantic_claim_fingerprint(claim: &ClaimV1) -> String {
    #[derive(Serialize)]
    struct FingerprintInput<'a> {
        subject: &'a ClaimEntity,
        assertion: ClaimAssertion,
        position: (i32, i32, Option<i32>),
        within_scene_order: Option<u32>,
        in_world_time: &'a Option<StoryClock>,
        modality: ClaimModality,
        temporal_scope: TemporalScope,
        speaker: &'a Option<ClaimEntity>,
        experiencer: &'a Option<ClaimEntity>,
    }
    let bytes = serde_json::to_vec(&FingerprintInput {
        subject: &claim.proposed.subject,
        assertion: normalized_assertion(&claim.proposed.assertion),
        position: (
            claim.binding.position.book_number,
            claim.binding.position.chapter_number,
            claim.binding.position.scene_order,
        ),
        within_scene_order: claim.proposed.within_scene_order,
        in_world_time: &claim.proposed.in_world_time,
        modality: claim.proposed.modality,
        temporal_scope: claim.proposed.temporal_scope,
        speaker: &claim.proposed.speaker,
        experiencer: &claim.proposed.experiencer,
    })
    .expect("semantic fingerprint input serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Deterministic semantic fingerprint for an event payload + position + subject.
pub fn semantic_event_fingerprint(
    subject: &ClaimEntity,
    payload: &CanonEventPayloadV1,
    effective_position: Option<CanonPositionV1>,
    within_scene_order: Option<u32>,
    in_world_time: &Option<StoryClock>,
) -> String {
    let normalized_payload = match payload {
        CanonEventPayloadV1::Assertion {
            assertion,
            modality,
            temporal_scope,
            speaker,
            experiencer,
        } => CanonEventPayloadV1::Assertion {
            assertion: normalized_assertion(assertion),
            modality: *modality,
            temporal_scope: *temporal_scope,
            speaker: speaker.clone(),
            experiencer: experiencer.clone(),
        },
        other => other.clone(),
    };
    #[derive(Serialize)]
    struct EventFingerprint<'a> {
        subject: &'a ClaimEntity,
        payload: CanonEventPayloadV1,
        effective_position: Option<CanonPositionV1>,
        within_scene_order: Option<u32>,
        in_world_time: &'a Option<StoryClock>,
    }
    let bytes = serde_json::to_vec(&EventFingerprint {
        subject,
        payload: normalized_payload,
        effective_position,
        within_scene_order,
        in_world_time,
    })
    .expect("event fingerprint serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Unique idempotency key: `branch + source revision + semantic fingerprint + policy action`.
pub fn compute_idempotency_key(
    branch_id: &str,
    source_revision_id: &str,
    semantic_fingerprint: &str,
    policy_action: CanonPolicyAction,
) -> String {
    #[derive(Serialize)]
    struct Key<'a> {
        branch_id: &'a str,
        source_revision_id: &'a str,
        semantic_fingerprint: &'a str,
        policy_action: CanonPolicyAction,
    }
    let bytes = serde_json::to_vec(&Key {
        branch_id,
        source_revision_id,
        semantic_fingerprint,
        policy_action,
    })
    .expect("idempotency key serializes");
    format!("{:x}", Sha256::digest(bytes))
}

/// Combine multiple event semantic fingerprints into a deterministic batch fingerprint
/// for the enclosing `CanonDecisionV1`.
pub fn batch_semantic_fingerprint(fingerprints: &[String]) -> String {
    let bytes = serde_json::to_vec(fingerprints).expect("fingerprint list serializes");
    format!("{:x}", Sha256::digest(bytes))
}

pub fn is_policy_actor(actor: &str) -> bool {
    let trimmed = actor.trim().to_ascii_lowercase();
    trimmed.starts_with("policy:") || trimmed == "auto" || trimmed == "system_policy"
}

fn entity_is_unresolved(entity: &ClaimEntity) -> bool {
    matches!(entity, ClaimEntity::Unresolved { .. })
}

pub fn assertion_has_unresolved_entity(
    subject: &ClaimEntity,
    assertion: &ClaimAssertion,
    speaker: &Option<ClaimEntity>,
    experiencer: &Option<ClaimEntity>,
) -> bool {
    if entity_is_unresolved(subject)
        || speaker.as_ref().is_some_and(entity_is_unresolved)
        || experiencer.as_ref().is_some_and(entity_is_unresolved)
    {
        return true;
    }
    match assertion {
        ClaimAssertion::LifeStatus { .. }
        | ClaimAssertion::Action { .. }
        | ClaimAssertion::SceneClock { .. }
        | ClaimAssertion::Injury { .. }
        | ClaimAssertion::Quantity { .. }
        | ClaimAssertion::Attribute { .. } => false,
        ClaimAssertion::LocatedAt { location } => entity_is_unresolved(location),
        ClaimAssertion::Move { from, to } => entity_is_unresolved(from) || entity_is_unresolved(to),
        ClaimAssertion::Possession {
            item, counterparty, ..
        } => entity_is_unresolved(item) || counterparty.as_ref().is_some_and(entity_is_unresolved),
        ClaimAssertion::Knowledge {
            proposition,
            audience,
            source,
            ..
        } => {
            entity_is_unresolved(proposition)
                || audience.iter().any(entity_is_unresolved)
                || source.as_ref().is_some_and(entity_is_unresolved)
        }
        ClaimAssertion::Relationship { other, .. } => entity_is_unresolved(other),
        ClaimAssertion::Promise { thread, .. } => entity_is_unresolved(thread),
        ClaimAssertion::Rule { rule, anchor, .. } => {
            entity_is_unresolved(rule) || anchor.as_ref().is_some_and(entity_is_unresolved)
        }
    }
}

/// V1 ratification policy guard: deaths/resurrections, secret disclosures,
/// promise payoffs/abandonments, invariant retcons, new world laws, temporal
/// interventions, and unresolved entities never auto-ratify in v1.
pub fn disallowed_for_v1_auto_ratification(
    subject: &ClaimEntity,
    assertion: &ClaimAssertion,
    speaker: &Option<ClaimEntity>,
    experiencer: &Option<ClaimEntity>,
    prior_life_status: Option<LifeStatus>,
    prior_invariant_differs: bool,
) -> Option<&'static str> {
    if assertion_has_unresolved_entity(subject, assertion, speaker, experiencer) {
        return Some("unresolved_entity");
    }
    match assertion {
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        } => Some("death_never_auto_ratifies_in_v1"),
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Alive,
        } if prior_life_status == Some(LifeStatus::Dead) => {
            Some("resurrection_never_auto_ratifies_in_v1")
        }
        ClaimAssertion::Knowledge {
            operation: KnowledgeOperation::Disclose,
            ..
        } => Some("secret_disclosure_never_auto_ratifies_in_v1"),
        ClaimAssertion::Promise {
            operation: PromiseOperation::Payoff | PromiseOperation::Abandon,
            ..
        } => Some("promise_payoff_never_auto_ratifies_in_v1"),
        ClaimAssertion::Attribute {
            nature: AttributeNature::Invariant,
            ..
        } if prior_invariant_differs => Some("invariant_retcon_never_auto_ratifies_in_v1"),
        ClaimAssertion::Rule {
            operation: RuleOperation::Define | RuleOperation::Intervene,
            ..
        } => Some("world_law_or_intervention_never_auto_ratifies_in_v1"),
        _ => None,
    }
}

/// Draft event specification supplied when appending a decision batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonEventDraftV1 {
    pub subject: ClaimEntity,
    pub payload: CanonEventPayloadV1,
    pub effective_position: Option<CanonPositionV1>,
    pub within_scene_order: Option<u32>,
    pub in_world_time: Option<StoryClock>,
    pub scene_id: Option<String>,
    pub source_claim_ids: Vec<String>,
    pub evidence_sha256: String,
    pub provenance: CanonProvenanceKind,
    pub supersedes_event_id: Option<String>,
    pub withdraws_event_id: Option<String>,
    pub legacy_record_scope: Option<LegacyRecordScopeV1>,
    pub merge_provenance: Option<MergeEventProvenanceV1>,
}

impl CanonEventDraftV1 {
    pub fn from_claim(
        claim: &ClaimV1,
        provenance: CanonProvenanceKind,
    ) -> Result<Self, CanonAppendError> {
        let pos = CanonPositionV1::from_placement(&claim.binding.position)?;
        Ok(Self {
            subject: claim.proposed.subject.clone(),
            payload: CanonEventPayloadV1::Assertion {
                assertion: claim.proposed.assertion.clone(),
                modality: claim.proposed.modality,
                temporal_scope: claim.proposed.temporal_scope,
                speaker: claim.proposed.speaker.clone(),
                experiencer: claim.proposed.experiencer.clone(),
            },
            effective_position: Some(pos),
            within_scene_order: claim.proposed.within_scene_order,
            in_world_time: claim.proposed.in_world_time.clone(),
            scene_id: claim.binding.scene_id.clone(),
            source_claim_ids: vec![claim.proposed.claim_id.clone()],
            evidence_sha256: evidence_sha256(&claim.proposed.evidence),
            provenance,
            supersedes_event_id: None,
            withdraws_event_id: None,
            legacy_record_scope: None,
            merge_provenance: None,
        })
    }
}

/// Input to append an atomic canon decision + event batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppendCanonDecisionInputV1 {
    pub project_id: String,
    pub branch_id: String,
    /// When true and `target_scene_id` is set, resolves inherited scene branch to
    /// the project's active descendant branch if it inherits from `branch_id`.
    pub resolve_active_descendant_branch: bool,
    pub action: CanonPolicyAction,
    pub tier: CanonTier,
    pub actor: String,
    pub policy_version: String,
    pub source_revision_id: String,
    pub target_scene_id: Option<String>,
    /// When true and `target_scene_id` is set, atomically invalidates prior active
    /// `source_revision_id`s for that scene on the target branch so recommitting a
    /// revised scene replaces rather than accumulates prior delta/collection effects.
    pub replace_prior_scene_revisions: bool,
    pub previous_decision_id: Option<String>,
    pub confidence_inputs: Vec<f64>,
    pub reason: String,
    pub events: Vec<CanonEventDraftV1>,
}

/// Output of an atomic canon decision + event append.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppendCanonDecisionOutputV1 {
    pub decision: CanonDecisionV1,
    pub events: Vec<CanonEventV1>,
    /// True when the idempotency key matched an already-committed decision/event batch.
    pub deduplicated: bool,
}

/// Materialized projection row stored in `canon_projection_state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonProjectionRowV1 {
    pub project_id: String,
    pub branch_id: String,
    pub tier: CanonTier,
    pub projection_kind: CanonProjectionKind,
    pub stream_key: String,
    pub subject_id: String,
    pub decision_watermark: i64,
    pub event_watermark: i64,
    pub reducer_version: u32,
    pub source_event_ids: Vec<String>,
    pub data_json: String,
    pub updated_at_micros: i64,
}
