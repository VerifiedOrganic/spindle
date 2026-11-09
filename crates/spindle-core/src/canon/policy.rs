//! Versioned, calibrated canon ratification policy (P2 PR E).
//!
//! This module is pure: it never writes canon and never calls a model. It turns
//! evidence-bound claims into a deterministic [`CanonPolicyEvaluationV1`] that
//! adapters persist as an audited `CanonDecisionV1`. V1 safety rules are hard
//! guards, not confidence weights: deaths/resurrections, secret disclosures,
//! promise payoffs/abandonments, invariant retcons, new world laws and temporal
//! interventions always require an author decision.

use super::comparison::{key_text, normalized_assertion};
use super::*;
use crate::models::StoryClock;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A versioned policy may only auto-ratify a class after held-out calibration
/// reaches this many accepted claims, ≥99% point precision, and a reported
/// 95% Wilson lower bound of at least 95%.
pub const CANON_POLICY_MIN_CALIBRATION_SAMPLE: usize = 100;
pub const CANON_POLICY_MIN_PRECISION_POINT: f64 = 0.99;
pub const CANON_POLICY_MIN_PRECISION_LOWER_BOUND: f64 = 0.95;
/// Two-sided 95% normal quantile used by the Wilson interval.
pub const CANON_POLICY_CALIBRATION_Z: f64 = 1.959_963_984_540_054;
pub const CANON_POLICY_EXPLICIT_THRESHOLD: f64 = 0.98;
pub const CANON_POLICY_ORDINARY_THRESHOLD: f64 = 0.99;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CanonClaimClassV1 {
    /// Explicit movement, scene clock, physical action, item custody.
    ExplicitMovement,
    /// Nonfatal injury add/heal.
    NonfatalInjury,
    /// Explicit quantity transaction with units.
    QuantityTransaction,
    /// Ordinary knowledge acquisition (not a secret disclosure).
    OrdinaryKnowledge,
    /// Relationship observation.
    RelationshipObservation,
    /// Evolving or conditional attribute observation.
    EvolvingAttribute,
    /// Promise plant/reinforcement on an existing thread.
    PromiseThread,
    /// Never auto-ratifies in v1.
    DeathOrResurrection,
    /// Never auto-ratifies in v1.
    SecretDisclosure,
    /// Never auto-ratifies in v1.
    PromisePayoffOrAbandonment,
    /// Never auto-ratifies in v1.
    InvariantRetcon,
    /// Never auto-ratifies in v1.
    WorldLawOrIntervention,
    /// Unresolved entity/time/units or an otherwise unsafe observation.
    AmbiguousOrUnsupported,
}

impl CanonClaimClassV1 {
    pub const ALL: [Self; 13] = [
        Self::ExplicitMovement,
        Self::NonfatalInjury,
        Self::QuantityTransaction,
        Self::OrdinaryKnowledge,
        Self::RelationshipObservation,
        Self::EvolvingAttribute,
        Self::PromiseThread,
        Self::DeathOrResurrection,
        Self::SecretDisclosure,
        Self::PromisePayoffOrAbandonment,
        Self::InvariantRetcon,
        Self::WorldLawOrIntervention,
        Self::AmbiguousOrUnsupported,
    ];

    /// Classes that can never auto-ratify in v1 regardless of score/agreement.
    pub fn is_v1_high_impact(self) -> bool {
        matches!(
            self,
            Self::DeathOrResurrection
                | Self::SecretDisclosure
                | Self::PromisePayoffOrAbandonment
                | Self::InvariantRetcon
                | Self::WorldLawOrIntervention
                | Self::AmbiguousOrUnsupported
        )
    }

    /// Minimum calibrated model confidence for unattended ratification. `None`
    /// means author decision only.
    pub fn auto_ratify_threshold(self, config: &CanonPolicyConfigV1) -> Option<f64> {
        match self {
            Self::ExplicitMovement | Self::NonfatalInjury | Self::QuantityTransaction => {
                Some(config.explicit_threshold)
            }
            Self::OrdinaryKnowledge | Self::RelationshipObservation | Self::EvolvingAttribute => {
                Some(config.ordinary_threshold)
            }
            Self::PromiseThread => Some(config.explicit_threshold),
            Self::DeathOrResurrection
            | Self::SecretDisclosure
            | Self::PromisePayoffOrAbandonment
            | Self::InvariantRetcon
            | Self::WorldLawOrIntervention
            | Self::AmbiguousOrUnsupported => None,
        }
    }

    /// True when the class is subject to the calibrated-sample gate.
    pub fn requires_calibration(self) -> bool {
        self.auto_ratify_threshold(&CanonPolicyConfigV1::default())
            .is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyConfigV1 {
    pub policy_version: String,
    pub explicit_threshold: f64,
    pub ordinary_threshold: f64,
    pub minimum_calibration_sample: usize,
    pub minimum_precision_point: f64,
    pub minimum_precision_lower_bound: f64,
    pub calibration_z: f64,
    /// Temporary escape hatch for tests/benchmarks only; default is `false`.
    /// Production policy must not auto-ratify an uncalibrated class.
    pub allow_uncalibrated_auto_ratification: bool,
}

impl Default for CanonPolicyConfigV1 {
    fn default() -> Self {
        Self {
            policy_version: CANON_POLICY_VERSION_V1.to_string(),
            explicit_threshold: CANON_POLICY_EXPLICIT_THRESHOLD,
            ordinary_threshold: CANON_POLICY_ORDINARY_THRESHOLD,
            minimum_calibration_sample: CANON_POLICY_MIN_CALIBRATION_SAMPLE,
            minimum_precision_point: CANON_POLICY_MIN_PRECISION_POINT,
            minimum_precision_lower_bound: CANON_POLICY_MIN_PRECISION_LOWER_BOUND,
            calibration_z: CANON_POLICY_CALIBRATION_Z,
            allow_uncalibrated_auto_ratification: false,
        }
    }
}

impl CanonPolicyConfigV1 {
    pub fn validate(&self) -> Result<(), CanonPolicyError> {
        if self.policy_version.trim().is_empty() {
            return Err(policy_error("policy_version", "must not be empty"));
        }
        for (name, value) in [
            ("explicit_threshold", self.explicit_threshold),
            ("ordinary_threshold", self.ordinary_threshold),
            ("minimum_precision_point", self.minimum_precision_point),
            (
                "minimum_precision_lower_bound",
                self.minimum_precision_lower_bound,
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(policy_error(name, "must be a finite number in [0,1]"));
            }
        }
        if !self.calibration_z.is_finite() || !(0.0..=10.0).contains(&self.calibration_z) {
            return Err(policy_error(
                "calibration_z",
                "must be a finite normal quantile in [0,10]",
            ));
        }
        if self.minimum_calibration_sample == 0 {
            return Err(policy_error(
                "minimum_calibration_sample",
                "must be at least 1",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptedClaimRevisionV1 {
    /// Exact accepted revision bytes; evidence spans must address these bytes.
    pub text: String,
    pub accepted_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimEvidenceFailureV1 {
    MissingRevision { revision_id: String },
    RevisionHashMismatch { revision_id: String },
    EmptyEvidence,
    SpanOutOfBounds { evidence_index: usize },
    QuoteMismatch { evidence_index: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimEvidenceVerificationV1 {
    pub claim_id: String,
    pub revision_id: String,
    pub verified: bool,
    pub failures: Vec<ClaimEvidenceFailureV1>,
}

/// Verify every claim's evidence against the exact accepted revision bytes and
/// hash. A missing revision is a failure, never an unverified pass.
pub fn verify_claim_batch_evidence(
    claims: &[ClaimV1],
    accepted_revisions: &BTreeMap<String, AcceptedClaimRevisionV1>,
) -> Vec<ClaimEvidenceVerificationV1> {
    claims
        .iter()
        .map(|claim| {
            let revision_id = claim.binding.revision_id.clone();
            let mut failures = Vec::new();
            match accepted_revisions.get(&revision_id) {
                None => failures.push(ClaimEvidenceFailureV1::MissingRevision {
                    revision_id: revision_id.clone(),
                }),
                Some(revision) => {
                    if revision.accepted_text_sha256 != claim.binding.accepted_text_sha256
                        || exact_text_sha256(&revision.text) != claim.binding.accepted_text_sha256
                    {
                        failures.push(ClaimEvidenceFailureV1::RevisionHashMismatch {
                            revision_id: revision_id.clone(),
                        });
                    }
                    if claim.proposed.evidence.is_empty() {
                        failures.push(ClaimEvidenceFailureV1::EmptyEvidence);
                    }
                    for (evidence_index, evidence) in claim.proposed.evidence.iter().enumerate() {
                        if evidence.start_byte >= evidence.end_byte {
                            failures
                                .push(ClaimEvidenceFailureV1::SpanOutOfBounds { evidence_index });
                            continue;
                        }
                        match revision.text.get(evidence.start_byte..evidence.end_byte) {
                            Some(span) if span == evidence.quote.as_str() => {}
                            _ => failures
                                .push(ClaimEvidenceFailureV1::QuoteMismatch { evidence_index }),
                        }
                    }
                }
            }
            ClaimEvidenceVerificationV1 {
                claim_id: claim.proposed.claim_id.clone(),
                revision_id,
                verified: failures.is_empty(),
                failures,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonContradictionSeverityV1 {
    Error,
    Warning,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonContradictionV1 {
    pub severity: CanonContradictionSeverityV1,
    pub stream: String,
    pub detail: String,
}

/// Compare a candidate claim against ratified canon. Missing state is
/// uncertainty, never an error; only explicit conflicting ratified values and
/// acting after a ratified death are error-level.
pub fn detect_canon_contradictions(
    claim: &ClaimV1,
    ratified: Option<&CanonStateViewV1>,
) -> Vec<CanonContradictionV1> {
    let Some(ratified) = ratified else {
        return Vec::new();
    };
    let ClaimEntity::Resolved { id: subject_id } = &claim.proposed.subject else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match &claim.proposed.assertion {
        ClaimAssertion::LifeStatus { status } => {
            let prior = ratified.life_status(subject_id);
            if prior != LifeStatus::Unknown && prior != *status {
                out.push(CanonContradictionV1 {
                    severity: CanonContradictionSeverityV1::Error,
                    stream: format!("life_status:{subject_id}"),
                    detail: format!("ratified life status is {prior:?}, candidate is {status:?}"),
                });
            }
        }
        ClaimAssertion::Action { mode, .. } => {
            if *mode == ActionMode::Physical && ratified.life_status(subject_id) == LifeStatus::Dead
            {
                out.push(CanonContradictionV1 {
                    severity: CanonContradictionSeverityV1::Error,
                    stream: format!("life_status:{subject_id}"),
                    detail: "physical action by a subject with ratified dead status".into(),
                });
            }
        }
        ClaimAssertion::Attribute {
            key,
            value,
            operation: AttributeOperation::Set,
            ..
        } => {
            let norm_key = key_text(key);
            if let Some(prior) = ratified
                .entity(subject_id)
                .and_then(|entity| entity.attributes.get(&norm_key))
            {
                if let Some(prior_value) = &prior.scalar_value {
                    if *prior_value != normalized_scalar(value) {
                        out.push(CanonContradictionV1 {
                            severity: CanonContradictionSeverityV1::Error,
                            stream: format!("attribute:{subject_id}:{norm_key}"),
                            detail: "ratified attribute has a different scalar value".into(),
                        });
                    }
                } else if !prior.members.is_empty() {
                    out.push(CanonContradictionV1 {
                        severity: CanonContradictionSeverityV1::Warning,
                        stream: format!("attribute:{subject_id}:{norm_key}"),
                        detail: "ratified attribute is a member set; scalar observation is not comparable".into(),
                    });
                }
            }
        }
        ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            amount,
            unit,
            ..
        } => {
            let measure = match &claim.proposed.assertion {
                ClaimAssertion::Quantity { measure, .. } => key_text(measure),
                _ => unreachable!(),
            };
            if let Some(balance) = ratified.quantity_balance(subject_id, &measure)
                && let (Ok(balance), Ok(delta), Some(prior_unit)) = (
                    ExactDecimal::parse(balance),
                    ExactDecimal::parse(amount),
                    ratified
                        .quantity(subject_id, &measure)
                        .map(|quantity| quantity.unit.as_str()),
                )
            {
                if prior_unit != unit.trim() {
                    out.push(CanonContradictionV1 {
                        severity: CanonContradictionSeverityV1::Error,
                        stream: format!("quantity:{subject_id}:{measure}"),
                        detail: "quantity transaction unit differs from the ratified stream".into(),
                    });
                } else if balance
                    .checked_add(delta)
                    .map(|next| next.is_negative())
                    .unwrap_or(false)
                {
                    out.push(CanonContradictionV1 {
                        severity: CanonContradictionSeverityV1::Error,
                        stream: format!("quantity:{subject_id}:{measure}"),
                        detail: "quantity transaction would spend more than the ratified balance"
                            .into(),
                    });
                }
            }
        }
        ClaimAssertion::Knowledge {
            operation: KnowledgeOperation::Use,
            proposition,
            ..
        } => {
            let proposition_id = entity_key(proposition);
            match ratified
                .knowledge
                .get(&proposition_id)
                .and_then(|knowledge| knowledge.knowers.get(subject_id))
            {
                None => out.push(CanonContradictionV1 {
                    severity: CanonContradictionSeverityV1::Unknown,
                    stream: format!("knowledge:{proposition_id}:{subject_id}"),
                    detail: "no ratified knowledge edge; acquisition order unknown".into(),
                }),
                Some(edge) if edge.status != KnowledgeStatus::Known => {
                    out.push(CanonContradictionV1 {
                        severity: CanonContradictionSeverityV1::Error,
                        stream: format!("knowledge:{proposition_id}:{subject_id}"),
                        detail: "subject uses knowledge that ratified canon says they do not know"
                            .into(),
                    })
                }
                Some(_) => {}
            }
        }
        _ => {}
    }
    out
}

fn normalized_scalar(value: &ClaimScalar) -> ClaimScalar {
    let assertion = ClaimAssertion::Attribute {
        operation: AttributeOperation::Set,
        key: "value".into(),
        value: value.clone(),
        nature: AttributeNature::Unknown,
    };
    match normalized_assertion(&assertion) {
        ClaimAssertion::Attribute { value, .. } => value,
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonCalibrationProfileV1 {
    pub class: CanonClaimClassV1,
    pub model: String,
    pub prompt_version: String,
    /// Held-out claims the policy would have auto-accepted.
    pub accepted_claims: usize,
    /// Accepted claims that matched the human adjudication.
    pub correct_claims: usize,
}

impl CanonCalibrationProfileV1 {
    /// Wilson score lower bound for the accepted-claim precision.
    pub fn precision_lower_bound(&self, z: f64) -> f64 {
        if self.accepted_claims == 0 {
            return 0.0;
        }
        let n = self.accepted_claims as f64;
        let p = (self.correct_claims.min(self.accepted_claims)) as f64 / n;
        let z2 = z * z;
        let denominator = 1.0 + z2 / n;
        let centre = p + z2 / (2.0 * n);
        let margin = z * ((p * (1.0 - p) / n) + z2 / (4.0 * n * n)).sqrt();
        ((centre - margin) / denominator).clamp(0.0, 1.0)
    }

    pub fn meets_gate(&self, config: &CanonPolicyConfigV1) -> bool {
        if self.accepted_claims < config.minimum_calibration_sample || self.accepted_claims == 0 {
            return false;
        }
        let precision =
            self.correct_claims.min(self.accepted_claims) as f64 / self.accepted_claims as f64;
        precision >= config.minimum_precision_point
            && self.precision_lower_bound(config.calibration_z)
                >= config.minimum_precision_lower_bound
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyContextV1 {
    pub accepted_revisions: BTreeMap<String, AcceptedClaimRevisionV1>,
    /// Ratified canon reduced through the scene cursor. `None` means no prior canon.
    pub ratified_state: Option<CanonStateViewV1>,
    /// Semantic fingerprints on which the independent extractor and drafter package agree.
    pub package_agreed_fingerprints: BTreeSet<String>,
    /// Secret proposition ids (or proposition ids linked to a secret canonical fact).
    pub secret_linked_propositions: BTreeSet<String>,
    /// Existing promise threads eligible for explicit plant/reinforcement.
    pub existing_promise_thread_ids: BTreeSet<String>,
    pub calibrations: Vec<CanonCalibrationProfileV1>,
    pub config: CanonPolicyConfigV1,
}

impl CanonPolicyContextV1 {
    pub fn calibration_for(
        &self,
        class: CanonClaimClassV1,
        model: &str,
        prompt: &str,
    ) -> Option<&CanonCalibrationProfileV1> {
        self.calibrations.iter().find(|profile| {
            profile.class == class && profile.model == model && profile.prompt_version == prompt
        })
    }
}

/// Where the claim came from. `OperatorDeclared` is an explicit author write
/// tool, not an inferred extraction source; the same shape is persisted as
/// `CanonProvenanceKind::ManualAuthor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonPolicyProvenanceV1 {
    IndependentExtracted,
    DrafterPackage,
    OperatorDeclared,
    LegacyUnverified,
    ImportedSeeded,
    Merged,
    SystemRestoration,
}

impl CanonPolicyProvenanceV1 {
    pub fn from_provenance(kind: CanonProvenanceKind) -> Self {
        match kind {
            CanonProvenanceKind::Extracted => Self::IndependentExtracted,
            CanonProvenanceKind::DrafterPackage => Self::DrafterPackage,
            CanonProvenanceKind::ManualAuthor => Self::OperatorDeclared,
            CanonProvenanceKind::LegacyUnverified => Self::LegacyUnverified,
            CanonProvenanceKind::Merged => Self::Merged,
            CanonProvenanceKind::SystemRestoration => Self::SystemRestoration,
        }
    }

    pub fn to_provenance(self) -> CanonProvenanceKind {
        match self {
            Self::IndependentExtracted => CanonProvenanceKind::Extracted,
            Self::DrafterPackage => CanonProvenanceKind::DrafterPackage,
            Self::OperatorDeclared => CanonProvenanceKind::ManualAuthor,
            Self::LegacyUnverified => CanonProvenanceKind::LegacyUnverified,
            Self::ImportedSeeded => CanonProvenanceKind::ManualAuthor,
            Self::Merged => CanonProvenanceKind::Merged,
            Self::SystemRestoration => CanonProvenanceKind::SystemRestoration,
        }
    }
}

/// Public disposition vocabulary. Persistence maps `AutoRatify` and `Ratify`
/// to `CanonPolicyAction::Ratify`; tier + actor distinguish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonPolicyDispositionV1 {
    AutoRatify,
    Ratify,
    StageProvisional,
    Reject,
    Supersede,
    Withdraw,
    InvalidateSource,
}

impl CanonPolicyDispositionV1 {
    pub fn policy_action(self) -> CanonPolicyAction {
        match self {
            Self::AutoRatify | Self::Ratify => CanonPolicyAction::Ratify,
            Self::StageProvisional => CanonPolicyAction::StageProvisional,
            Self::Reject => CanonPolicyAction::Reject,
            Self::Supersede => CanonPolicyAction::Supersede,
            Self::Withdraw => CanonPolicyAction::Withdraw,
            Self::InvalidateSource => CanonPolicyAction::InvalidateSource,
        }
    }

    pub fn tier(self) -> CanonTier {
        match self {
            Self::AutoRatify | Self::Ratify => CanonTier::Ratified,
            Self::StageProvisional => CanonTier::Provisional,
            Self::Reject => CanonTier::Provisional,
            Self::Supersede => CanonTier::Ratified,
            Self::Withdraw => CanonTier::Ratified,
            Self::InvalidateSource => CanonTier::Ratified,
        }
    }

    pub fn claim_status(self) -> CanonClaimStatus {
        match self {
            Self::AutoRatify | Self::Ratify => CanonClaimStatus::Ratified,
            Self::StageProvisional => CanonClaimStatus::Provisional,
            Self::Reject => CanonClaimStatus::Rejected,
            Self::Supersede | Self::Withdraw => CanonClaimStatus::Withdrawn,
            Self::InvalidateSource => CanonClaimStatus::SourceInvalidated,
        }
    }

    pub fn is_unattended(self) -> bool {
        self == Self::AutoRatify
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyPreconditionV1 {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyEvaluationV1 {
    pub claim_id: String,
    pub semantic_fingerprint: String,
    pub class: CanonClaimClassV1,
    pub disposition: CanonPolicyDispositionV1,
    pub action: CanonPolicyAction,
    pub tier: CanonTier,
    pub actor: String,
    pub policy_version: String,
    pub confidence_inputs: Vec<f64>,
    pub preconditions: Vec<CanonPolicyPreconditionV1>,
    pub precondition_summary: String,
    pub reason: String,
    /// Deduplicates retries of this claim/action at the same revision/branch.
    pub idempotency_key: String,
    pub contradictions: Vec<CanonContradictionV1>,
    pub evidence_failures: Vec<ClaimEvidenceFailureV1>,
    pub calibrated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyError {
    pub field: String,
    pub detail: String,
}

impl std::fmt::Display for CanonPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.detail)
    }
}
impl std::error::Error for CanonPolicyError {}

fn policy_error(field: impl Into<String>, detail: impl Into<String>) -> CanonPolicyError {
    CanonPolicyError {
        field: field.into(),
        detail: detail.into(),
    }
}

impl From<CanonAppendError> for CanonPolicyError {
    fn from(value: CanonAppendError) -> Self {
        Self {
            field: value.field,
            detail: value.detail,
        }
    }
}

/// Classify a claim using only the candidate and ratified state. Secret linkage
/// is supplied by the caller because it can come from a canonical-fact secret
/// edge rather than the claim text itself.
pub fn classify_canon_claim(
    claim: &ClaimV1,
    ratified: Option<&CanonStateViewV1>,
    secret_linked: bool,
    existing_promise_thread_ids: &BTreeSet<String>,
) -> CanonClaimClassV1 {
    if assertion_has_unresolved_entity(
        &claim.proposed.subject,
        &claim.proposed.assertion,
        &claim.proposed.speaker,
        &claim.proposed.experiencer,
    ) {
        return CanonClaimClassV1::AmbiguousOrUnsupported;
    }
    let subject_id = match &claim.proposed.subject {
        ClaimEntity::Resolved { id } => id.as_str(),
        ClaimEntity::Unresolved { .. } => return CanonClaimClassV1::AmbiguousOrUnsupported,
    };
    match &claim.proposed.assertion {
        ClaimAssertion::LifeStatus { status } => match status {
            LifeStatus::Dead => CanonClaimClassV1::DeathOrResurrection,
            LifeStatus::Alive
                if ratified
                    .is_some_and(|state| state.life_status(subject_id) == LifeStatus::Dead) =>
            {
                CanonClaimClassV1::DeathOrResurrection
            }
            _ => CanonClaimClassV1::ExplicitMovement,
        },
        ClaimAssertion::Action { mode, .. } => match mode {
            ActionMode::Physical => CanonClaimClassV1::ExplicitMovement,
            ActionMode::Speech
            | ActionMode::Ghost
            | ActionMode::Recording
            | ActionMode::Unknown => CanonClaimClassV1::AmbiguousOrUnsupported,
        },
        ClaimAssertion::LocatedAt { .. }
        | ClaimAssertion::Move { .. }
        | ClaimAssertion::SceneClock { .. }
        | ClaimAssertion::Possession { .. } => CanonClaimClassV1::ExplicitMovement,
        ClaimAssertion::Injury { .. } => CanonClaimClassV1::NonfatalInjury,
        ClaimAssertion::Quantity { .. } => CanonClaimClassV1::QuantityTransaction,
        ClaimAssertion::Knowledge { operation, .. } => {
            if *operation == KnowledgeOperation::Disclose || secret_linked {
                CanonClaimClassV1::SecretDisclosure
            } else {
                CanonClaimClassV1::OrdinaryKnowledge
            }
        }
        ClaimAssertion::Relationship { operation, .. } => match operation {
            RelationshipOperation::Observation => CanonClaimClassV1::RelationshipObservation,
            RelationshipOperation::Delta => CanonClaimClassV1::RelationshipObservation,
        },
        ClaimAssertion::Attribute {
            nature, key, value, ..
        } => {
            let invariant_differs = ratified
                .and_then(|state| state.entity(subject_id))
                .and_then(|entity| entity.attributes.get(&key_text(key)))
                .is_some_and(|attribute| {
                    attribute.nature == AttributeNature::Invariant
                        && attribute.scalar_value.as_ref() != Some(&normalized_scalar(value))
                });
            match nature {
                AttributeNature::Invariant if invariant_differs => {
                    CanonClaimClassV1::InvariantRetcon
                }
                AttributeNature::Invariant => CanonClaimClassV1::EvolvingAttribute,
                AttributeNature::Evolving | AttributeNature::Conditional => {
                    CanonClaimClassV1::EvolvingAttribute
                }
                AttributeNature::Unknown => CanonClaimClassV1::AmbiguousOrUnsupported,
            }
        }
        ClaimAssertion::Promise { operation, thread } => match operation {
            PromiseOperation::Payoff | PromiseOperation::Abandon => {
                CanonClaimClassV1::PromisePayoffOrAbandonment
            }
            PromiseOperation::Plant | PromiseOperation::Reinforce => {
                let thread_id = entity_key(thread);
                if thread_id.starts_with("unresolved:") {
                    CanonClaimClassV1::AmbiguousOrUnsupported
                } else if existing_promise_thread_ids.contains(&thread_id) {
                    CanonClaimClassV1::PromiseThread
                } else {
                    CanonClaimClassV1::AmbiguousOrUnsupported
                }
            }
        },
        ClaimAssertion::Rule { operation, .. } => match operation {
            RuleOperation::Use | RuleOperation::Anchor => CanonClaimClassV1::ExplicitMovement,
            RuleOperation::Define | RuleOperation::Intervene => {
                CanonClaimClassV1::WorldLawOrIntervention
            }
        },
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonDecisionAuditV1 {
    pub policy_version: String,
    pub semantic_fingerprint: String,
    pub idempotency_key: String,
    pub confidence_inputs: Vec<f64>,
    pub precondition_summary: String,
}

impl From<&CanonPolicyEvaluationV1> for CanonDecisionAuditV1 {
    fn from(evaluation: &CanonPolicyEvaluationV1) -> Self {
        Self {
            policy_version: evaluation.policy_version.clone(),
            semantic_fingerprint: evaluation.semantic_fingerprint.clone(),
            idempotency_key: evaluation.idempotency_key.clone(),
            confidence_inputs: evaluation.confidence_inputs.clone(),
            precondition_summary: evaluation.precondition_summary.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPolicyClaimRequestV1 {
    pub claim: ClaimV1,
    pub provenance: CanonProvenanceKind,
    /// Secret-fact linkage discovered from legacy canon, not inferred from prose.
    pub secret_linked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordCanonPolicyInputV1 {
    pub project_id: String,
    pub branch_id: String,
    pub source_revision_id: String,
    pub target_scene_id: Option<String>,
    /// Set on the first decision of a batch when the caller evaluated against a known tip.
    pub previous_decision_id: Option<String>,
    pub replace_prior_scene_revisions: bool,
    pub requests: Vec<CanonPolicyClaimRequestV1>,
    pub context: CanonPolicyContextV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordCanonPolicyOutputV1 {
    pub evaluations: Vec<CanonPolicyEvaluationV1>,
    pub claims: Vec<CanonClaimRecordV1>,
    pub decisions: Vec<AppendCanonDecisionOutputV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeCanonDecisionInputV1 {
    pub project_id: String,
    pub branch_id: String,
    pub decision_id: String,
    pub actor: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeCanonDecisionOutputV1 {
    pub plan: CanonRevocationPlanV1,
    pub decision: AppendCanonDecisionOutputV1,
    pub invalidated_claim_ids: Vec<String>,
    pub invalidated_event_ids: Vec<String>,
    pub invalidated_summary_keys: Vec<String>,
}

/// A modality is eligible for unattended world truth only when it is direct
/// narration of the present; all belief/speech/dream/negation/flashback forms
/// stay provisional.
pub fn modality_is_auto_eligible(claim: &ClaimV1) -> bool {
    claim.proposed.modality == ClaimModality::Narration
        && claim.proposed.temporal_scope == TemporalScope::Current
}

fn push_precondition(
    out: &mut Vec<CanonPolicyPreconditionV1>,
    name: &str,
    passed: bool,
    detail: impl Into<String>,
) {
    out.push(CanonPolicyPreconditionV1 {
        name: name.into(),
        passed,
        detail: detail.into(),
    });
}

fn summarize_preconditions(preconditions: &[CanonPolicyPreconditionV1]) -> String {
    preconditions
        .iter()
        .map(|precondition| {
            format!(
                "{}={}",
                precondition.name,
                if precondition.passed { "pass" } else { "fail" }
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Evaluate one claim. This is deterministic and side-effect free.
pub fn evaluate_canon_policy(
    claim: &ClaimV1,
    provenance: CanonPolicyProvenanceV1,
    secret_linked: bool,
    context: &CanonPolicyContextV1,
) -> Result<CanonPolicyEvaluationV1, CanonPolicyError> {
    context.config.validate()?;
    if claim.proposed.claim_id.trim().is_empty() {
        return Err(policy_error("claim_id", "must not be empty"));
    }
    if claim
        .proposed
        .confidence
        .is_none_or(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
    {
        return Err(policy_error(
            "confidence",
            "policy requires finite confidence in [0,1]",
        ));
    }
    let confidence = claim.proposed.confidence.expect("checked above");
    let fingerprint = semantic_claim_fingerprint(claim);
    let class = classify_canon_claim(
        claim,
        context.ratified_state.as_ref(),
        secret_linked,
        &context.existing_promise_thread_ids,
    );
    let evidence =
        verify_claim_batch_evidence(std::slice::from_ref(claim), &context.accepted_revisions)
            .into_iter()
            .next()
            .expect("one claim yields one verification");
    let contradictions = detect_canon_contradictions(claim, context.ratified_state.as_ref());
    let has_error_contradiction = contradictions
        .iter()
        .any(|item| item.severity == CanonContradictionSeverityV1::Error);
    let operator_declared = provenance == CanonPolicyProvenanceV1::OperatorDeclared;
    let agreement = context.package_agreed_fingerprints.contains(&fingerprint);
    let calibration =
        context.calibration_for(class, &claim.binding.model, &claim.binding.prompt_version);
    let calibrated = calibration.is_some_and(|profile| profile.meets_gate(&context.config))
        || context.config.allow_uncalibrated_auto_ratification;
    let calibration_lower_bound = calibration
        .map(|profile| profile.precision_lower_bound(context.config.calibration_z))
        .unwrap_or(0.0);

    let mut preconditions = Vec::new();
    push_precondition(
        &mut preconditions,
        "evidence",
        evidence.verified,
        if evidence.verified {
            "all UTF-8 spans match the accepted revision hash and bytes"
        } else {
            "evidence spans do not verify against the accepted revision"
        },
    );
    let resolved_entity = !assertion_has_unresolved_entity(
        &claim.proposed.subject,
        &claim.proposed.assertion,
        &claim.proposed.speaker,
        &claim.proposed.experiencer,
    );
    push_precondition(
        &mut preconditions,
        "entity_resolution",
        resolved_entity,
        if resolved_entity {
            "subject and referenced entities are resolved ids"
        } else {
            "at least one entity is unresolved"
        },
    );
    push_precondition(
        &mut preconditions,
        "no_error_contradiction",
        !has_error_contradiction,
        if has_error_contradiction {
            "candidate contradicts error-level ratified canon"
        } else {
            "no error-level contradiction against ratified canon"
        },
    );
    let modality_eligible = modality_is_auto_eligible(claim);
    push_precondition(
        &mut preconditions,
        "modality",
        modality_eligible,
        if modality_eligible {
            "direct present narration"
        } else {
            "dialogue/belief/speculation/dream/negation/flashback stays provisional"
        },
    );
    let agreement_eligible = agreement || operator_declared;
    push_precondition(
        &mut preconditions,
        "source_agreement",
        agreement_eligible,
        if operator_declared {
            "explicit operator declaration bypasses package agreement"
        } else if agreement {
            "package and independent extractor agree"
        } else {
            "package and independent extractor do not agree"
        },
    );
    push_precondition(
        &mut preconditions,
        "calibration",
        calibrated,
        if calibrated {
            "class/model/prompt calibration meets the held-out sample and precision gate"
        } else {
            "insufficient held-out calibration; class stays shadow only"
        },
    );
    let threshold = class.auto_ratify_threshold(&context.config);
    push_precondition(
        &mut preconditions,
        "confidence_threshold",
        threshold.is_some_and(|threshold| confidence >= threshold),
        match threshold {
            Some(threshold) => format!("confidence {confidence} >= {threshold}"),
            None => "high-impact class has no unattended threshold".into(),
        },
    );

    let safe_for_auto = !class.is_v1_high_impact()
        && evidence.verified
        && resolved_entity
        && !has_error_contradiction
        && modality_eligible
        && agreement
        && calibrated
        && threshold.is_some_and(|threshold| confidence >= threshold);
    let eligible_for_operator = resolved_entity
        && (!has_error_contradiction || operator_declared)
        && !class.is_v1_high_impact()
        && modality_eligible;

    let (disposition, actor) = if class.is_v1_high_impact() {
        (
            CanonPolicyDispositionV1::StageProvisional,
            "policy:canon-policy-v1".to_string(),
        )
    } else if safe_for_auto {
        (
            CanonPolicyDispositionV1::AutoRatify,
            "policy:canon-policy-v1".to_string(),
        )
    } else if operator_declared && eligible_for_operator {
        (
            CanonPolicyDispositionV1::Ratify,
            "operator:canon-policy-v1".to_string(),
        )
    } else {
        (
            CanonPolicyDispositionV1::StageProvisional,
            "policy:canon-policy-v1".to_string(),
        )
    };

    let reason = match disposition {
        CanonPolicyDispositionV1::AutoRatify => "all calibrated unattended-ratification preconditions passed",
        CanonPolicyDispositionV1::Ratify => "explicit operator declaration ratified the claim",
        CanonPolicyDispositionV1::StageProvisional
            if class.is_v1_high_impact() =>
        {
            "v1 safety rule: death/resurrection, secret disclosure, promise payoff/abandonment, invariant retcon, world-law change, temporal intervention and ambiguous claims never auto-ratify"
        }
        CanonPolicyDispositionV1::StageProvisional => {
            "one or more unattended-ratification preconditions failed; staged for author decision"
        }
        _ => "policy disposition",
    }
    .to_string();

    let confidence_inputs = vec![confidence, calibration_lower_bound];
    Ok(CanonPolicyEvaluationV1 {
        claim_id: claim.proposed.claim_id.clone(),
        semantic_fingerprint: fingerprint.clone(),
        class,
        disposition,
        action: disposition.policy_action(),
        tier: disposition.tier(),
        actor,
        policy_version: context.config.policy_version.clone(),
        confidence_inputs,
        precondition_summary: summarize_preconditions(&preconditions),
        preconditions,
        reason,
        idempotency_key: compute_idempotency_key(
            &claim.binding.branch_id,
            &claim.binding.revision_id,
            &fingerprint,
            disposition.policy_action(),
        ),
        contradictions,
        evidence_failures: evidence.failures,
        calibrated,
    })
}

/// Collect the independent extractor's fingerprints for claims where the
/// drafter package agrees. This is the only agreement signal the policy trusts.
pub fn agreed_claim_fingerprints(comparisons: &[ClaimComparisonV1]) -> BTreeSet<String> {
    comparisons
        .iter()
        .filter(|comparison| comparison.kind == ClaimComparisonKind::Agreed)
        .flat_map(|comparison| comparison.extractor.iter())
        .map(semantic_claim_fingerprint)
        .collect()
}

/// Evaluate a persisted claim record using its recorded provenance. This is the
/// bridge from `CanonClaimRecordV1` to the same pure policy used for new claims.
pub fn evaluate_canon_claim_record(
    record: &CanonClaimRecordV1,
    context: &CanonPolicyContextV1,
    secret_linked: bool,
) -> Result<CanonPolicyEvaluationV1, CanonPolicyError> {
    evaluate_canon_policy(
        &record.claim,
        CanonPolicyProvenanceV1::from_provenance(record.provenance),
        secret_linked,
        context,
    )
}

/// Evaluate a batch; duplicate claim ids or malformed confidence reject the
/// batch rather than partially applying it.
pub fn evaluate_canon_policy_batch(
    claims: &[ClaimV1],
    provenance: CanonPolicyProvenanceV1,
    secret_linked_propositions: &BTreeSet<String>,
    context: &CanonPolicyContextV1,
) -> Result<Vec<CanonPolicyEvaluationV1>, CanonPolicyError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(claims.len());
    for claim in claims {
        if !seen.insert(claim.proposed.claim_id.clone()) {
            return Err(policy_error(
                "claim_id",
                format!("duplicate claim id {}", claim.proposed.claim_id),
            ));
        }
        let secret_linked = match &claim.proposed.assertion {
            ClaimAssertion::Knowledge { proposition, .. } => {
                secret_linked_propositions.contains(&entity_key(proposition))
            }
            _ => false,
        };
        out.push(evaluate_canon_policy(
            claim,
            provenance,
            secret_linked,
            context,
        )?);
    }
    Ok(out)
}

/// Group a batch by disposition/action so adapters can persist one audited
/// decision per resulting action without reordering claims.
pub fn group_policy_evaluations_by_action(
    evaluations: &[CanonPolicyEvaluationV1],
) -> BTreeMap<CanonPolicyAction, Vec<&CanonPolicyEvaluationV1>> {
    let mut grouped: BTreeMap<CanonPolicyAction, Vec<&CanonPolicyEvaluationV1>> = BTreeMap::new();
    for evaluation in evaluations {
        grouped
            .entry(evaluation.action)
            .or_default()
            .push(evaluation);
    }
    grouped
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPendingWorkInputV1 {
    pub project_id: String,
    pub branch_id: String,
    /// Accepted scenes whose eligible claims have no final policy decision.
    pub pending_scene_count: usize,
    /// Disputed high-impact claims the next scene depends on.
    pub disputed_high_impact_prerequisites: usize,
    /// Scenes whose extraction failed/timed out and never completed.
    pub failed_extraction_scene_count: usize,
    /// Two retries inside the 60-second scene budget before pending warnings.
    pub retries_remaining: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonPendingWorkV1 {
    pub project_id: String,
    pub branch_id: String,
    pub pending_scene_count: usize,
    pub disputed_high_impact_prerequisites: usize,
    pub failed_extraction_scene_count: usize,
    pub next_scene_has_pending_work: bool,
    pub pause_at_next_checkpoint: bool,
    pub reasons: Vec<String>,
}

impl CanonPendingWorkV1 {
    /// Checkpoint/continuity-clean reporting must be false while extraction or
    /// policy decisions are pending. Never silently promote a backlog.
    pub fn continuity_clean(&self) -> bool {
        !self.next_scene_has_pending_work
            && !self.pause_at_next_checkpoint
            && self.pending_scene_count == 0
            && self.failed_extraction_scene_count == 0
            && self.disputed_high_impact_prerequisites == 0
    }
}

pub fn evaluate_canon_pending_work(input: &CanonPendingWorkInputV1) -> CanonPendingWorkV1 {
    let mut reasons = Vec::new();
    if input.pending_scene_count > 3 {
        reasons.push("more than three accepted scenes have pending policy decisions".into());
    }
    if input.disputed_high_impact_prerequisites > 0 {
        reasons.push("the next scene depends on a disputed high-impact claim".into());
    }
    if input.failed_extraction_scene_count > 0 {
        reasons.push("extraction failed/timed out for at least one accepted scene".into());
    }
    let pause_at_next_checkpoint =
        input.pending_scene_count > 3 || input.disputed_high_impact_prerequisites > 0;
    CanonPendingWorkV1 {
        project_id: input.project_id.clone(),
        branch_id: input.branch_id.clone(),
        pending_scene_count: input.pending_scene_count,
        disputed_high_impact_prerequisites: input.disputed_high_impact_prerequisites,
        failed_extraction_scene_count: input.failed_extraction_scene_count,
        next_scene_has_pending_work: input.pending_scene_count > 0
            || input.failed_extraction_scene_count > 0,
        pause_at_next_checkpoint,
        reasons,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonRevocationPlanV1 {
    pub branch_id: String,
    pub source_revision_id: String,
    pub withdraw_event_drafts: Vec<CanonEventDraftV1>,
    /// Claim records that lose automatic ratification until revalidated.
    pub invalidated_claim_ids: Vec<String>,
    /// Downstream events that depended on the revoked batch.
    pub invalidated_dependent_event_ids: Vec<String>,
    /// Derived memory keys (scene/chapter/arc/book summaries) to invalidate.
    pub invalidated_summary_keys: Vec<String>,
    pub semantic_fingerprint: String,
}

/// Plan revocation of a bad decision batch: append one `Withdraw` event per
/// target, then invalidate dependent claims/events and derived summaries. The
/// audit records and original events remain.
pub fn plan_canon_batch_revocation(
    target_events: &[CanonEventV1],
    all_events: &[CanonEventV1],
    claims: &[CanonClaimRecordV1],
    branch_id: &str,
    source_revision_id: &str,
) -> CanonRevocationPlanV1 {
    let target_event_ids: BTreeSet<&str> = target_events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    let target_claim_ids: BTreeSet<&str> = target_events
        .iter()
        .flat_map(|event| event.source_claim_ids.iter().map(String::as_str))
        .collect();
    let mut dependent_event_ids = BTreeSet::new();
    for event in all_events {
        if target_event_ids.contains(event.event_id.as_str()) {
            continue;
        }
        let shares_claim = event
            .source_claim_ids
            .iter()
            .any(|claim_id| target_claim_ids.contains(claim_id.as_str()));
        let same_stream = target_events.iter().any(|target| {
            target.subject_key() == event.subject_key()
                && target.predicate == event.predicate
                && event.decision_seq >= target.decision_seq
        });
        let explicitly_dependent = event
            .supersedes_event_id
            .as_deref()
            .is_some_and(|id| target_event_ids.contains(id))
            || event
                .withdraws_event_id
                .as_deref()
                .is_some_and(|id| target_event_ids.contains(id));
        if shares_claim || same_stream || explicitly_dependent {
            dependent_event_ids.insert(event.event_id.clone());
        }
    }
    let invalidated_claim_ids = claims
        .iter()
        .filter(|claim| {
            target_claim_ids.contains(claim.claim.proposed.claim_id.as_str())
                || target_event_ids
                    .iter()
                    .any(|event_id| claim.claim_record_id.contains(event_id))
        })
        .map(|claim| claim.claim_record_id.clone())
        .collect::<Vec<_>>();
    let withdraw_event_drafts = target_events
        .iter()
        .map(|event| CanonEventDraftV1 {
            subject: event.subject.clone(),
            payload: event.payload.clone(),
            effective_position: event.effective_position,
            within_scene_order: event.within_scene_order,
            in_world_time: event.in_world_time.clone(),
            scene_id: event.scene_id.clone(),
            source_claim_ids: event.source_claim_ids.clone(),
            evidence_sha256: event.evidence_sha256.clone(),
            provenance: CanonProvenanceKind::SystemRestoration,
            supersedes_event_id: None,
            withdraws_event_id: Some(event.event_id.clone()),
            legacy_record_scope: event.legacy_record_scope.clone(),
            merge_provenance: None,
        })
        .collect();
    let mut summary_keys = target_events
        .iter()
        .filter_map(|event| event.scene_id.as_ref())
        .map(|scene_id| format!("scene:{scene_id}"))
        .collect::<BTreeSet<_>>();
    if let Some(first) = target_events.first()
        && let Some(position) = first.effective_position
    {
        summary_keys.insert(format!(
            "chapter:{}:{}",
            position.book_number, position.chapter_number
        ));
    }
    let fingerprints = target_events
        .iter()
        .map(|event| event.semantic_fingerprint.clone())
        .collect::<Vec<_>>();
    CanonRevocationPlanV1 {
        branch_id: branch_id.to_string(),
        source_revision_id: source_revision_id.to_string(),
        withdraw_event_drafts,
        invalidated_claim_ids,
        invalidated_dependent_event_ids: dependent_event_ids.into_iter().collect(),
        invalidated_summary_keys: summary_keys.into_iter().collect(),
        semantic_fingerprint: batch_semantic_fingerprint(&fingerprints),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonCutoverModeV1 {
    /// Legacy tables are authoritative; no canon mirroring.
    Legacy,
    /// Legacy write + canon append in one transaction.
    DualWrite,
    /// Dual write plus compatibility shadow-read comparison.
    ShadowRead,
    /// Canon projections are authoritative for reads.
    CanonRead,
}

impl CanonCutoverModeV1 {
    pub fn mirrors_writes(self) -> bool {
        matches!(self, Self::DualWrite | Self::ShadowRead | Self::CanonRead)
    }

    pub fn reads_canon(self) -> bool {
        self == Self::CanonRead
    }

    pub fn shadows_reads(self) -> bool {
        self == Self::ShadowRead
    }

    fn rank(self) -> u8 {
        match self {
            Self::Legacy => 0,
            Self::DualWrite => 1,
            Self::ShadowRead => 2,
            Self::CanonRead => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonCutoverStateV1 {
    pub project_id: String,
    pub mode: CanonCutoverModeV1,
    /// Decision watermark at activation; rollback can reproduce the prior view.
    pub activated_decision_watermark: i64,
    pub reason: String,
    pub updated_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonCutoverError {
    pub project_id: String,
    pub requested_mode: CanonCutoverModeV1,
    pub detail: String,
}

impl std::fmt::Display for CanonCutoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot cut project {} over to {:?}: {}",
            self.project_id, self.requested_mode, self.detail
        )
    }
}
impl std::error::Error for CanonCutoverError {}

/// Per-project cutover gate. Forward transitions require clean compatibility
/// parity and no unexplained/quarantined rows; rollbacks are always allowed so
/// the feature flag is reversible.
pub fn evaluate_canon_cutover_transition(
    current: Option<&CanonCutoverStateV1>,
    requested: CanonCutoverModeV1,
    readiness: &ProjectCutoverReadinessV1,
    decision_watermark: i64,
    now_micros: i64,
) -> Result<CanonCutoverStateV1, CanonCutoverError> {
    let current_mode = current
        .map(|state| state.mode)
        .unwrap_or(CanonCutoverModeV1::Legacy);
    // Any rollback to a lower mode is allowed (the feature flag is reversible);
    // forward movement must pass through each stage.
    let allowed = requested == current_mode
        || requested.rank() < current_mode.rank()
        || matches!(
            (current_mode, requested),
            (CanonCutoverModeV1::Legacy, CanonCutoverModeV1::DualWrite)
                | (
                    CanonCutoverModeV1::DualWrite,
                    CanonCutoverModeV1::ShadowRead
                )
                | (
                    CanonCutoverModeV1::ShadowRead,
                    CanonCutoverModeV1::CanonRead
                )
        );
    if !allowed {
        return Err(CanonCutoverError {
            project_id: readiness.project_id.clone(),
            requested_mode: requested,
            detail: format!("transition from {current_mode:?} to {requested:?} is not allowed"),
        });
    }
    if requested != CanonCutoverModeV1::Legacy && !readiness.cutover_allowed {
        return Err(CanonCutoverError {
            project_id: readiness.project_id.clone(),
            requested_mode: requested,
            detail: format!(
                "cutover readiness blocked: {}",
                readiness.blocking_reasons.join("; ")
            ),
        });
    }
    Ok(CanonCutoverStateV1 {
        project_id: readiness.project_id.clone(),
        mode: requested,
        activated_decision_watermark: decision_watermark,
        reason: if requested == CanonCutoverModeV1::Legacy {
            "rolled back to legacy authority".into()
        } else {
            "compatibility parity and quarantine gates passed".into()
        },
        updated_at_micros: now_micros,
    })
}

/// `LegacyUnverified` rows participate in compatibility reads and lexical
/// prefilters, but cannot produce an error-level semantic finding until an
/// evidence-verified claim or operator ratification corroborates them.
pub fn can_trigger_error_finding(provenance: CanonProvenanceKind, corroborated: bool) -> bool {
    provenance != CanonProvenanceKind::LegacyUnverified || corroborated
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LegacyCanonFindingV1 {
    pub severity: String,
    pub check_type: String,
    pub message: String,
    pub provenance: CanonProvenanceKind,
    pub corroborated: bool,
}

/// Downgrade a `LegacyUnverified` semantic finding to a warning unless it has
/// evidence-verified or operator-ratified corroboration.
pub fn classify_legacy_finding(finding: LegacyCanonFindingV1) -> LegacyCanonFindingV1 {
    if finding.severity == "error"
        && !can_trigger_error_finding(finding.provenance, finding.corroborated)
    {
        LegacyCanonFindingV1 {
            severity: "warning".into(),
            message: format!(
                "legacy-unverified evidence only; requires corroboration before blocking: {}",
                finding.message
            ),
            ..finding
        }
    } else {
        finding
    }
}

/// Narrative position of a claim, or an error when placement is missing.
pub fn claim_position(claim: &ClaimV1) -> Result<CanonPositionV1, CanonPolicyError> {
    CanonPositionV1::from_placement(&claim.binding.position).map_err(Into::into)
}

/// In-world clock is preserved but never used as narrative placement.
pub fn claim_clock(claim: &ClaimV1) -> Option<&StoryClock> {
    claim.proposed.in_world_time.as_ref()
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
