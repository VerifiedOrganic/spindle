//! Evidence-bound extraction contracts. These are proposals, never canon writes.
mod comparison;
mod validation;
use crate::models::{StoryClock, StoryPlacement};
pub use comparison::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use validation::*;

pub const CLAIM_SCHEMA_VERSION: u32 = 1;
pub const MAX_EXTRACTION_PAYLOAD_BYTES: usize = 32_000;
pub const EXTRACT_PROMPT_VERSION: &str = "canon-extract-v1";

macro_rules! vocabulary {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
    };
}
mod backfill;
mod events;
mod reducer;
pub use backfill::*;
pub use events::*;
pub use reducer::*;
vocabulary!(ClaimFamily {
    LifeAction,
    LocationClock,
    Possession,
    Injury,
    Quantity,
    KnowledgeDisclosure,
    AttributeWorldState,
    Relationship,
    Promise,
    RuleTemporal
});
pub const CLAIM_FAMILIES: [ClaimFamily; 10] = [
    ClaimFamily::LifeAction,
    ClaimFamily::LocationClock,
    ClaimFamily::Possession,
    ClaimFamily::Injury,
    ClaimFamily::Quantity,
    ClaimFamily::KnowledgeDisclosure,
    ClaimFamily::AttributeWorldState,
    ClaimFamily::Relationship,
    ClaimFamily::Promise,
    ClaimFamily::RuleTemporal,
];
vocabulary!(LifeStatus {
    Alive,
    Dead,
    Unknown
});
vocabulary!(ActionMode {
    Physical,
    Speech,
    Ghost,
    Recording,
    Unknown
});
vocabulary!(PossessionOperation {
    Acquire,
    Transfer,
    Lose
});
vocabulary!(CustodyKind { Owner, Custodian });
vocabulary!(InjuryOperation { Add, Heal });
vocabulary!(InjurySeverity {
    Minor,
    Moderate,
    Severe,
    Unknown
});
vocabulary!(QuantityOperation { Set, Delta });
vocabulary!(KnowledgeOperation {
    Learn,
    Forget,
    Use,
    Disclose
});
vocabulary!(AttributeNature {
    Invariant,
    Evolving,
    Conditional,
    Unknown
});
vocabulary!(AttributeOperation { Set, Add, Remove });
vocabulary!(RelationshipOperation { Delta, Observation });
vocabulary!(PromiseOperation {
    Plant,
    Reinforce,
    Payoff,
    Abandon
});
vocabulary!(RuleOperation {
    Define,
    Use,
    Anchor,
    Intervene
});
vocabulary!(ClaimModality {
    Narration,
    Dialogue,
    Belief,
    Speculation,
    Dream,
    Negated
});
vocabulary!(TemporalScope {
    Current,
    Flashback,
    Future,
    Unknown
});
vocabulary!(ClaimSource {
    Independent,
    Drafter
});
vocabulary!(ExtractionValidationCode {
    Identity,
    Hash,
    Schema,
    Coverage,
    Evidence,
    Entity,
    Value,
    Confidence,
    DuplicateClaim,
    Limit
});
vocabulary!(ClaimComparisonKind {
    Agreed,
    ExtractorOnly,
    DrafterOnly,
    Incompatible
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "resolution", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimEntity {
    Resolved { id: String },
    Unresolved { label: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimScalar {
    Text { value: String },
    Boolean { value: bool },
    Decimal { value: String, unit: Option<String> },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "predicate", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaimAssertion {
    LifeStatus {
        status: LifeStatus,
    },
    Action {
        verb: String,
        mode: ActionMode,
    },
    LocatedAt {
        location: ClaimEntity,
    },
    Move {
        from: ClaimEntity,
        to: ClaimEntity,
    },
    SceneClock {
        clock: StoryClock,
    },
    Possession {
        operation: PossessionOperation,
        item: ClaimEntity,
        counterparty: Option<ClaimEntity>,
        custody: CustodyKind,
    },
    Injury {
        operation: InjuryOperation,
        injury_id: String,
        body_site: String,
        severity: InjurySeverity,
    },
    Quantity {
        operation: QuantityOperation,
        measure: String,
        amount: String,
        unit: String,
        transaction_id: Option<String>,
    },
    Knowledge {
        operation: KnowledgeOperation,
        proposition: ClaimEntity,
        audience: Vec<ClaimEntity>,
        source: Option<ClaimEntity>,
    },
    Attribute {
        operation: AttributeOperation,
        key: String,
        value: ClaimScalar,
        nature: AttributeNature,
    },
    Relationship {
        operation: RelationshipOperation,
        other: ClaimEntity,
        trust: i32,
        tension: i32,
    },
    Promise {
        operation: PromiseOperation,
        thread: ClaimEntity,
    },
    Rule {
        operation: RuleOperation,
        rule: ClaimEntity,
        anchor: Option<ClaimEntity>,
        preconditions: Vec<String>,
        effects: Vec<String>,
    },
}
impl ClaimAssertion {
    pub fn family(&self) -> ClaimFamily {
        match self {
            Self::LifeStatus { .. } | Self::Action { .. } => ClaimFamily::LifeAction,
            Self::LocatedAt { .. } | Self::Move { .. } | Self::SceneClock { .. } => {
                ClaimFamily::LocationClock
            }
            Self::Possession { .. } => ClaimFamily::Possession,
            Self::Injury { .. } => ClaimFamily::Injury,
            Self::Quantity { .. } => ClaimFamily::Quantity,
            Self::Knowledge { .. } => ClaimFamily::KnowledgeDisclosure,
            Self::Attribute { .. } => ClaimFamily::AttributeWorldState,
            Self::Relationship { .. } => ClaimFamily::Relationship,
            Self::Promise { .. } => ClaimFamily::Promise,
            Self::Rule { .. } => ClaimFamily::RuleTemporal,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimEvidence {
    pub start_byte: usize,
    pub end_byte: usize,
    pub quote: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposedClaimV1 {
    pub claim_id: String,
    pub subject: ClaimEntity,
    pub assertion: ClaimAssertion,
    pub within_scene_order: Option<u32>,
    pub in_world_time: Option<StoryClock>,
    pub modality: ClaimModality,
    pub temporal_scope: TemporalScope,
    pub speaker: Option<ClaimEntity>,
    pub experiencer: Option<ClaimEntity>,
    pub confidence: Option<f64>,
    pub evidence: Vec<ClaimEvidence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum FamilyCoverage {
    InspectedNone,
    ClaimsPresent,
    Unknown { reason: String },
}
/// The model supplies proposals only; trusted identity/provenance is attached locally.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractionPayloadV1 {
    pub schema_version: u32,
    pub accepted_text_sha256: String,
    pub summary: String,
    pub tone: Option<String>,
    pub coverage: BTreeMap<ClaimFamily, FamilyCoverage>,
    pub claims: Vec<ProposedClaimV1>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimBindingV1 {
    pub schema_version: u32,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: Option<String>,
    /// Immutable accepted artifact/revision, not the mutable scene row.
    pub revision_id: String,
    pub accepted_text_sha256: String,
    pub position: StoryPlacement,
    pub extraction_run_id: String,
    pub model: String,
    pub agent_id: Option<String>,
    pub model_call_id: Option<String>,
    pub prompt_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimV1 {
    pub binding: ClaimBindingV1,
    pub source: ClaimSource,
    pub proposed: ProposedClaimV1,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionReportV1 {
    pub binding: ClaimBindingV1,
    pub summary: String,
    pub tone: Option<String>,
    pub coverage: BTreeMap<ClaimFamily, FamilyCoverage>,
    pub claims: Vec<ClaimV1>,
}

#[cfg(test)]
mod tests;

/// Provider schema for the proposed (unbound) payload, shared with host fallback.
pub fn extraction_payload_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ExtractionPayloadV1))
        .expect("claim schema serializes")
}

/// Durable shadow audit carried by P1's cached compatibility result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonExtractionAuditV1 {
    pub task_id: String,
    pub request_sha256: String,
    pub report: ExtractionReportV1,
}

impl ExtractionReportV1 {
    /// Inspection completeness only; this never certifies consistency or truth.
    pub fn coverage_is_complete(&self) -> bool {
        self.coverage.len() == CLAIM_FAMILIES.len()
            && CLAIM_FAMILIES.iter().all(|family| {
                self.coverage
                    .get(family)
                    .is_some_and(|status| !matches!(status, FamilyCoverage::Unknown { .. }))
            })
    }
}
