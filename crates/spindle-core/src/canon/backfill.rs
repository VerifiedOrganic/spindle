//! Backfill contracts, legacy compatibility shadow read reports, and cutover readiness gates.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::canon::{
    AttributeNature, AttributeOperation, CanonPositionV1, CanonSourceMapEntryV1, ClaimAssertion,
    ClaimEntity, ClaimScalar, InjuryOperation, InjurySeverity, LifeStatus, QuantityOperation,
};
use crate::models::StoryPlacement;

/// Default backfill migration version for Section 2's legacy-to-canon store mapping.
pub const CANON_BACKFILL_MIGRATION_VERSION_V1: u32 = 1;

/// Legacy stores covered by Section 2's backfill and shadow-read migration plan.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CanonBackfillStoreKind {
    CanonicalFact,
    CharacterState,
    WorldState,
    WorldRule,
    KnowledgeFact,
    Knows,
    FutureKnowledge,
    QuantityState,
    ProgressionEvent,
    SystemOverlay,
    TimelineEvent,
    TemporalIntervention,
    SceneClock,
    TimelineEventClock,
    NarrativePromise,
    Relationship,
    RelationshipSceneDelta,
    CanonDelta,
}

pub const ALL_BACKFILL_STORES: [CanonBackfillStoreKind; 18] = [
    CanonBackfillStoreKind::CanonicalFact,
    CanonBackfillStoreKind::CharacterState,
    CanonBackfillStoreKind::WorldState,
    CanonBackfillStoreKind::WorldRule,
    CanonBackfillStoreKind::KnowledgeFact,
    CanonBackfillStoreKind::Knows,
    CanonBackfillStoreKind::FutureKnowledge,
    CanonBackfillStoreKind::QuantityState,
    CanonBackfillStoreKind::ProgressionEvent,
    CanonBackfillStoreKind::SystemOverlay,
    CanonBackfillStoreKind::TimelineEvent,
    CanonBackfillStoreKind::TemporalIntervention,
    CanonBackfillStoreKind::SceneClock,
    CanonBackfillStoreKind::TimelineEventClock,
    CanonBackfillStoreKind::NarrativePromise,
    CanonBackfillStoreKind::Relationship,
    CanonBackfillStoreKind::RelationshipSceneDelta,
    CanonBackfillStoreKind::CanonDelta,
];

impl CanonBackfillStoreKind {
    pub fn as_source_table(self) -> &'static str {
        match self {
            Self::CanonicalFact => "canonical_fact",
            Self::CharacterState => "character_state",
            Self::WorldState => "world_state",
            Self::WorldRule => "world_rule",
            Self::KnowledgeFact => "knowledge_fact",
            Self::Knows => "knows",
            Self::FutureKnowledge => "future_knowledge",
            Self::QuantityState => "quantity_state",
            Self::ProgressionEvent => "progression_event",
            Self::SystemOverlay => "system_overlay",
            Self::TimelineEvent => "timeline_event",
            Self::TemporalIntervention => "temporal_intervention",
            Self::SceneClock => "scene_clock",
            Self::TimelineEventClock => "timeline_event_clock",
            Self::NarrativePromise => "narrative_promise",
            Self::Relationship => "relates_to",
            Self::RelationshipSceneDelta => "relationship_scene_delta",
            Self::CanonDelta => "canon_delta",
        }
    }
}

/// Per-store counts from a legacy-to-canon backfill run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonStoreBackfillCountsV1 {
    pub mapped_count: usize,
    pub duplicate_skipped_count: usize,
    pub quarantined_count: usize,
    pub reused_idempotent_count: usize,
    pub emitted_event_count: usize,
    pub emitted_claim_count: usize,
    pub emitted_mask_count: usize,
}

/// Summary of a project-wide backfill run across all branches and legacy stores.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonBackfillReportV1 {
    pub project_id: String,
    pub migration_version: u32,
    pub by_store: BTreeMap<String, CanonStoreBackfillCountsV1>,
    pub total_mapped_count: usize,
    pub total_duplicate_skipped_count: usize,
    pub total_quarantined_count: usize,
    pub total_reused_idempotent_count: usize,
    pub total_emitted_event_count: usize,
    pub total_emitted_claim_count: usize,
    pub total_emitted_mask_count: usize,
    pub quarantined_entries: Vec<CanonSourceMapEntryV1>,
}

impl CanonBackfillReportV1 {
    pub fn new(project_id: impl Into<String>, migration_version: u32) -> Self {
        let mut by_store = BTreeMap::new();
        for store in ALL_BACKFILL_STORES {
            by_store.insert(
                store.as_source_table().to_string(),
                CanonStoreBackfillCountsV1::default(),
            );
        }
        Self {
            project_id: project_id.into(),
            migration_version,
            by_store,
            total_mapped_count: 0,
            total_duplicate_skipped_count: 0,
            total_quarantined_count: 0,
            total_reused_idempotent_count: 0,
            total_emitted_event_count: 0,
            total_emitted_claim_count: 0,
            total_emitted_mask_count: 0,
            quarantined_entries: Vec::new(),
        }
    }

    pub fn record_store_counts(
        &mut self,
        store: CanonBackfillStoreKind,
        delta: &CanonStoreBackfillCountsV1,
    ) {
        let entry = self
            .by_store
            .entry(store.as_source_table().to_string())
            .or_default();
        entry.mapped_count += delta.mapped_count;
        entry.duplicate_skipped_count += delta.duplicate_skipped_count;
        entry.quarantined_count += delta.quarantined_count;
        entry.reused_idempotent_count += delta.reused_idempotent_count;
        entry.emitted_event_count += delta.emitted_event_count;
        entry.emitted_claim_count += delta.emitted_claim_count;
        entry.emitted_mask_count += delta.emitted_mask_count;

        self.total_mapped_count += delta.mapped_count;
        self.total_duplicate_skipped_count += delta.duplicate_skipped_count;
        self.total_quarantined_count += delta.quarantined_count;
        self.total_reused_idempotent_count += delta.reused_idempotent_count;
        self.total_emitted_event_count += delta.emitted_event_count;
        self.total_emitted_claim_count += delta.emitted_claim_count;
        self.total_emitted_mask_count += delta.emitted_mask_count;
    }
}

/// Single discrepancy between a legacy store/view read and its canon compatibility projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonShadowParityDifferenceV1 {
    pub store_or_view: String,
    pub branch_id: String,
    pub subject_id: Option<String>,
    pub natural_key: String,
    pub legacy_value: serde_json::Value,
    pub projected_value: serde_json::Value,
    pub detail: String,
}

/// Branch-scoped compatibility shadow-read comparison report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonCompatibilityShadowReportV1 {
    pub project_id: String,
    pub branch_id: String,
    pub placement: Option<StoryPlacement>,
    pub parity_clean: bool,
    pub checked_store_count: usize,
    pub checked_subject_snapshot_count: usize,
    pub differences: Vec<CanonShadowParityDifferenceV1>,
}

/// Per-project cutover readiness verdict enforcing Section 2's Stage B -> C gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectCutoverReadinessV1 {
    pub project_id: String,
    pub migration_version: u32,
    pub parity_clean: bool,
    pub quarantined_count: usize,
    pub unmapped_legacy_row_count: usize,
    pub cutover_allowed: bool,
    pub blocking_reasons: Vec<String>,
    pub branch_reports: Vec<CanonCompatibilityShadowReportV1>,
}

/// Converts raw `(book_number, chapter_number, scene_order)` into a `CanonPositionV1`
/// only when all coordinates are positive (`>= 1`). Unknown or baseline `(<= 0)` coordinates
/// map to `None` (baseline of unknown effective time, never scene zero).
pub fn coordinates_to_canon_position(
    book_number: Option<i32>,
    chapter_number: Option<i32>,
    scene_order: Option<i32>,
) -> Option<CanonPositionV1> {
    match (book_number, chapter_number, scene_order) {
        (Some(b), Some(c), Some(s)) if b >= 1 && c >= 1 && s >= 1 => {
            CanonPositionV1::new(b, c, s).ok()
        }
        _ => None,
    }
}

/// Converts a `StoryPlacement` into a `CanonPositionV1` when it has positive book, chapter,
/// and scene_order coordinates; otherwise returns `None` (never inventing scene zero).
pub fn story_placement_to_canon_position(placement: &StoryPlacement) -> Option<CanonPositionV1> {
    coordinates_to_canon_position(
        Some(placement.book_number),
        Some(placement.chapter_number),
        placement.scene_order,
    )
}

/// Produces a deterministic, collision-resistant proposition ID shared between `knowledge_fact` and `knows`.
pub fn deduplicated_knowledge_proposition_id(
    normalized_fact: &str,
    secret_of_fact_id: Option<&str>,
) -> String {
    if let Some(secret_id) = secret_of_fact_id.map(str::trim).filter(|s| !s.is_empty()) {
        format!("proposition:secret:{secret_id}")
    } else {
        use sha2::{Digest, Sha256};
        let canonical = normalized_fact.trim().to_lowercase();
        let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
        let slug: String = canonical
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '-')
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let collapsed: String = slug
            .split('_')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("_")
            .chars()
            .take(32)
            .collect();
        if collapsed.is_empty() {
            format!("proposition:fact:{digest}")
        } else {
            format!("proposition:fact:{collapsed}:{digest}")
        }
    }
}

/// Parses a clear injury status token such as `"injury:left_arm:severe"`, `"injured:shoulder"`,
/// or `"wounded:ribs:minor"` into `(injury_id, body_site, severity)`.
pub fn parse_clear_injury_status_token(
    character_id: &str,
    token: &str,
) -> Option<(String, String, InjurySeverity)> {
    let trimmed = token.trim();
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("injury:")
        .or_else(|| lower.strip_prefix("injured:"))
        .or_else(|| lower.strip_prefix("wounded:"))?;
    let parts: Vec<&str> = rest
        .split(':')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    let body_site = parts[0].replace(' ', "_");
    if body_site.is_empty() {
        return None;
    }
    let severity = match parts.get(1).copied() {
        Some("minor" | "light") => InjurySeverity::Minor,
        Some("moderate" | "medium") => InjurySeverity::Moderate,
        Some("severe" | "critical" | "grave" | "heavy") => InjurySeverity::Severe,
        Some(_) | None => InjurySeverity::Unknown,
    };
    let injury_id = format!("legacy_injury:{character_id}:{body_site}");
    Some((injury_id, body_site, severity))
}

fn is_dead_status_token(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "dead" | "deceased" | "killed" | "slain"
    )
}

fn is_alive_status_token(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "alive" | "living"
    )
}

/// Computes explicit per-field/member difference assertions between consecutive `character_state`
/// status snapshots on a branch. Clear life-status and injury tokens emit `LifeStatus` and `Injury`
/// assertions, while all raw status entries are also tracked as `character_state.status` set-member
/// attributes so ambiguous raw strings remain preserved as legacy annotations.
pub fn diff_character_status_snapshots(
    character_id: &str,
    previous_status: &[String],
    current_status: &[String],
) -> Vec<ClaimAssertion> {
    let prev_set: BTreeSet<&str> = previous_status.iter().map(String::as_str).collect();
    let curr_set: BTreeSet<&str> = current_status.iter().map(String::as_str).collect();

    let mut assertions = Vec::new();

    let had_dead = previous_status.iter().any(|s| is_dead_status_token(s));
    let has_dead = current_status.iter().any(|s| is_dead_status_token(s));
    let had_alive = previous_status.iter().any(|s| is_alive_status_token(s));
    let has_alive = current_status.iter().any(|s| is_alive_status_token(s));

    if has_dead && !had_dead {
        assertions.push(ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        });
    } else if has_alive && !had_alive && !has_dead {
        assertions.push(ClaimAssertion::LifeStatus {
            status: LifeStatus::Alive,
        });
    }

    let prev_injuries: BTreeMap<String, (String, InjurySeverity)> = previous_status
        .iter()
        .filter_map(|token| {
            parse_clear_injury_status_token(character_id, token)
                .map(|(id, site, sev)| (id, (site, sev)))
        })
        .collect();
    let curr_injuries: BTreeMap<String, (String, InjurySeverity)> = current_status
        .iter()
        .filter_map(|token| {
            parse_clear_injury_status_token(character_id, token)
                .map(|(id, site, sev)| (id, (site, sev)))
        })
        .collect();

    for (injury_id, (body_site, severity)) in &curr_injuries {
        if prev_injuries.get(injury_id) != Some(&(body_site.clone(), *severity)) {
            assertions.push(ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: injury_id.clone(),
                body_site: body_site.clone(),
                severity: *severity,
            });
        }
    }
    for (injury_id, (body_site, severity)) in &prev_injuries {
        if !curr_injuries.contains_key(injury_id) {
            assertions.push(ClaimAssertion::Injury {
                operation: InjuryOperation::Heal,
                injury_id: injury_id.clone(),
                body_site: body_site.clone(),
                severity: *severity,
            });
        }
    }

    for removed in previous_status {
        if !curr_set.contains(removed.as_str()) {
            assertions.push(ClaimAssertion::Attribute {
                operation: AttributeOperation::Remove,
                key: "character_state.status".to_string(),
                value: ClaimScalar::Text {
                    value: removed.clone(),
                },
                nature: AttributeNature::Evolving,
            });
        }
    }
    for added in current_status {
        if !prev_set.contains(added.as_str()) {
            assertions.push(ClaimAssertion::Attribute {
                operation: AttributeOperation::Add,
                key: "character_state.status".to_string(),
                value: ClaimScalar::Text {
                    value: added.clone(),
                },
                nature: AttributeNature::Evolving,
            });
        }
    }

    assertions
}

/// Classification outcome for a legacy `progression_event.delta_json` payload.
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressionDeltaClassification {
    Attributable {
        assertions: Vec<ClaimAssertion>,
        quantity_measures: BTreeSet<String>,
        touches_character_status: bool,
    },
    Quarantined {
        reason: String,
    },
}

/// Classifies a legacy `progression_event.delta_json` payload into typed canon assertions
/// or quarantines opaque/unattributable JSON.
pub fn classify_progression_delta_json(
    character_id: &str,
    event_type: &str,
    delta_json: &str,
) -> ProgressionDeltaClassification {
    let parsed: serde_json::Value = match serde_json::from_str(delta_json) {
        Ok(value) => value,
        Err(err) => {
            return ProgressionDeltaClassification::Quarantined {
                reason: format!("invalid progression delta_json: {err}"),
            };
        }
    };
    let Some(obj) = parsed.as_object() else {
        return ProgressionDeltaClassification::Quarantined {
            reason: "progression delta_json must be a JSON object".to_string(),
        };
    };
    if obj.is_empty() {
        return ProgressionDeltaClassification::Quarantined {
            reason: "progression delta_json is an empty object".to_string(),
        };
    }

    let mut assertions = Vec::new();
    let mut quantity_measures = BTreeSet::new();
    let mut touches_character_status = false;
    let mut unrecognized_keys = Vec::new();

    for (key, value) in obj {
        match key.as_str() {
            "tier" | "tier_to" | "to_tier" | "new_tier" | "rank" | "level" | "class" | "title" => {
                if let Some(text) = scalar_from_json_value(value) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("progression.{key}"),
                        value: text,
                        nature: AttributeNature::Evolving,
                    });
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "tier_from" | "from_tier" | "old_tier" | "reason" | "notes" => {
                if let Some(text) = scalar_from_json_value(value) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: format!("progression.{key}"),
                        value: text,
                        nature: AttributeNature::Evolving,
                    });
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "skill_added" | "ability_added" | "perk_added" => {
                if let Some(text) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    assertions.push(ClaimAssertion::Attribute {
                        operation: AttributeOperation::Add,
                        key: format!("progression.{key}"),
                        value: ClaimScalar::Text {
                            value: text.to_string(),
                        },
                        nature: AttributeNature::Evolving,
                    });
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "skills" | "abilities" => {
                if let Some(arr) = value.as_array() {
                    let mut added_any = false;
                    for item in arr {
                        if let Some(text) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            assertions.push(ClaimAssertion::Attribute {
                                operation: AttributeOperation::Add,
                                key: format!("progression.{key}"),
                                value: ClaimScalar::Text {
                                    value: text.to_string(),
                                },
                                nature: AttributeNature::Evolving,
                            });
                            added_any = true;
                        }
                    }
                    if !added_any {
                        unrecognized_keys.push(key.clone());
                    }
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "stats" | "stat_deltas" | "attributes" => {
                if let Some(stats_obj) = value.as_object() {
                    if stats_obj.is_empty() {
                        unrecognized_keys.push(key.clone());
                    }
                    for (stat_name, stat_val) in stats_obj {
                        if let Some(num) = numeric_string_from_json_value(stat_val) {
                            let op = if key == "stat_deltas" {
                                QuantityOperation::Delta
                            } else {
                                QuantityOperation::Set
                            };
                            quantity_measures.insert(stat_name.clone());
                            assertions.push(ClaimAssertion::Quantity {
                                operation: op,
                                measure: stat_name.clone(),
                                amount: num,
                                unit: "stat".to_string(),
                                transaction_id: None,
                            });
                        } else if let Some(scalar) = scalar_from_json_value(stat_val) {
                            assertions.push(ClaimAssertion::Attribute {
                                operation: AttributeOperation::Set,
                                key: format!("progression.stat.{stat_name}"),
                                value: scalar,
                                nature: AttributeNature::Evolving,
                            });
                        } else {
                            unrecognized_keys.push(format!("{key}.{stat_name}"));
                        }
                    }
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "quantity" => {
                if let Some(q_obj) = value.as_object() {
                    let measure = q_obj
                        .get("measure")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    let amount = q_obj.get("amount").and_then(numeric_string_from_json_value);
                    let unit = q_obj.get("unit").and_then(|v| v.as_str()).unwrap_or("unit");
                    let op = match q_obj.get("operation").and_then(|v| v.as_str()) {
                        Some("delta") => QuantityOperation::Delta,
                        _ => QuantityOperation::Set,
                    };
                    if let (Some(measure), Some(amount)) = (measure, amount) {
                        quantity_measures.insert(measure.to_string());
                        assertions.push(ClaimAssertion::Quantity {
                            operation: op,
                            measure: measure.to_string(),
                            amount,
                            unit: unit.to_string(),
                            transaction_id: None,
                        });
                    } else {
                        unrecognized_keys.push(key.clone());
                    }
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            "measure" | "amount" | "delta" | "unit" => {
                // Handled below as top-level measure+amount/delta tuple if present.
            }
            "status" => {
                if let Some(status_str) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    touches_character_status = true;
                    let status_vec = vec![status_str.to_string()];
                    assertions.extend(diff_character_status_snapshots(
                        character_id,
                        &[],
                        &status_vec,
                    ));
                } else if let Some(arr) = value.as_array() {
                    let status_vec: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect();
                    if status_vec.is_empty() {
                        unrecognized_keys.push(key.clone());
                    } else {
                        touches_character_status = true;
                        assertions.extend(diff_character_status_snapshots(
                            character_id,
                            &[],
                            &status_vec,
                        ));
                    }
                } else {
                    unrecognized_keys.push(key.clone());
                }
            }
            _ => {
                unrecognized_keys.push(key.clone());
            }
        }
    }

    if obj.contains_key("measure") || obj.contains_key("amount") || obj.contains_key("delta") {
        let measure = obj
            .get("measure")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let (amount, op) =
            if let Some(amt) = obj.get("amount").and_then(numeric_string_from_json_value) {
                (Some(amt), QuantityOperation::Set)
            } else if let Some(d) = obj.get("delta").and_then(numeric_string_from_json_value) {
                (Some(d), QuantityOperation::Delta)
            } else {
                (None, QuantityOperation::Set)
            };
        let unit = obj.get("unit").and_then(|v| v.as_str()).unwrap_or("unit");
        if let (Some(measure), Some(amount)) = (measure, amount) {
            quantity_measures.insert(measure.to_string());
            assertions.push(ClaimAssertion::Quantity {
                operation: op,
                measure: measure.to_string(),
                amount,
                unit: unit.to_string(),
                transaction_id: None,
            });
        } else {
            unrecognized_keys.push("measure/amount".to_string());
        }
    }

    if !unrecognized_keys.is_empty() || assertions.is_empty() {
        return ProgressionDeltaClassification::Quarantined {
            reason: format!(
                "opaque or unattributable progression_event ({event_type}) delta_json keys: {}",
                if unrecognized_keys.is_empty() {
                    "<no attributable assertions>".to_string()
                } else {
                    unrecognized_keys.join(", ")
                }
            ),
        };
    }

    ProgressionDeltaClassification::Attributable {
        assertions,
        quantity_measures,
        touches_character_status,
    }
}

fn scalar_from_json_value(value: &serde_json::Value) -> Option<ClaimScalar> {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => {
            Some(ClaimScalar::Text { value: s.clone() })
        }
        serde_json::Value::Bool(b) => Some(ClaimScalar::Boolean { value: *b }),
        serde_json::Value::Number(n) => Some(ClaimScalar::Decimal {
            value: n.to_string(),
            unit: None,
        }),
        _ => None,
    }
}

fn numeric_string_from_json_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) if s.trim().parse::<f64>().is_ok() => {
            Some(s.trim().to_string())
        }
        _ => None,
    }
}

/// Builds a resolved `ClaimEntity` for a legacy subject ID.
pub fn resolved_entity(id: impl Into<String>) -> ClaimEntity {
    ClaimEntity::Resolved { id: id.into() }
}
