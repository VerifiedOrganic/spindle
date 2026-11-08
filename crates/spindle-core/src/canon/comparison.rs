use super::*;
use crate::models::{CanonicalFactEntry, CharacterStatePatchEntry, RelationshipUpdateEntry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DrafterPackageV1 {
    pub character_states: Vec<CharacterStatePatchEntry>,
    pub canonical_facts: Vec<CanonicalFactEntry>,
    pub relationship_updates: Vec<RelationshipUpdateEntry>,
    pub continuity_notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NormalizedDrafterPackageV1 {
    pub claims: Vec<ClaimV1>,
    /// Field paths that cannot safely become typed assertions, not prose claims.
    pub unsupported_fields: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimComparisonV1 {
    pub kind: ClaimComparisonKind,
    pub extractor: Vec<ClaimV1>,
    pub drafter: Vec<ClaimV1>,
    pub recommended_resolution: String,
}
fn resolve(id: &str, known: &BTreeSet<String>) -> ClaimEntity {
    if known.contains(id) {
        ClaimEntity::Resolved { id: id.into() }
    } else {
        ClaimEntity::Unresolved { label: id.into() }
    }
}
fn drafter_scalar(fact: &CanonicalFactEntry) -> Option<ClaimScalar> {
    let text = fact.value_text.as_ref().or(fact.value.as_ref());
    if fact
        .value_text
        .as_ref()
        .zip(fact.value.as_ref())
        .is_some_and(|(a, b)| a != b)
    {
        return None;
    }
    let representations = usize::from(text.is_some())
        + usize::from(fact.value_number.is_some())
        + usize::from(fact.value_json.is_some());
    if representations != 1 {
        return None;
    }
    let kind = fact
        .value_kind
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let unit = fact
        .value_unit
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string());
    match kind.as_str() {
        "number" => {
            let value = if let Some(number) = fact.value_number.filter(|v| v.is_finite()) {
                number.to_string()
            } else {
                text?.clone()
            };
            super::validation::decimal(&value).ok()?;
            Some(ClaimScalar::Decimal { value, unit })
        }
        "boolean" | "bool" => {
            let value = match fact.value_json.as_ref() {
                Some(serde_json::Value::Bool(value)) => *value,
                Some(_) => return None,
                None => match text?.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => return None,
                },
            };
            Some(ClaimScalar::Boolean { value })
        }
        "" => {
            if let Some(number) = fact.value_number.filter(|v| v.is_finite()) {
                Some(ClaimScalar::Decimal {
                    value: number.to_string(),
                    unit,
                })
            } else if let Some(serde_json::Value::Bool(value)) = &fact.value_json {
                Some(ClaimScalar::Boolean { value: *value })
            } else {
                Some(ClaimScalar::Text {
                    value: text.filter(|s| !s.trim().is_empty())?.clone(),
                })
            }
        }
        "text" | "string" | "enum" => Some(ClaimScalar::Text {
            value: text.filter(|s| !s.trim().is_empty())?.clone(),
        }),
        _ => None,
    }
}
/// Lossy legacy fields are reported, never silently interpreted as typed canon.
pub fn normalize_drafter_package(
    package: &DrafterPackageV1,
    binding: &ClaimBindingV1,
    known: &BTreeSet<String>,
) -> NormalizedDrafterPackageV1 {
    let mut out = NormalizedDrafterPackageV1 {
        claims: Vec::new(),
        unsupported_fields: Vec::new(),
    };
    let mut add = |subject, assertion, position: Option<StoryPlacement>| {
        let mut binding = binding.clone();
        if let Some(position) = position {
            binding.position = position;
        }
        binding.extraction_run_id = format!("drafter:{}", binding.revision_id);
        binding.model = "drafter_package".into();
        binding.agent_id = None;
        binding.model_call_id = None;
        binding.prompt_version = "legacy-package-v1".into();
        out.claims.push(ClaimV1 {
            binding,
            source: ClaimSource::Drafter,
            proposed: ProposedClaimV1 {
                claim_id: format!("drafter:{}", out.claims.len()),
                subject,
                assertion,
                within_scene_order: None,
                in_world_time: None,
                modality: ClaimModality::Narration,
                temporal_scope: TemporalScope::Current,
                speaker: None,
                experiencer: None,
                confidence: None,
                evidence: Vec::new(),
            },
        });
    };
    for (i, entry) in package.character_states.iter().enumerate() {
        if let Some(statuses) = &entry.changes.status {
            out.unsupported_fields
                .push(format!("character_states[{i}].status_replacement"));
            for status in statuses {
                let assertion = match status.trim().to_ascii_lowercase().as_str() {
                    "dead" | "deceased" | "status:deceased" => ClaimAssertion::LifeStatus {
                        status: LifeStatus::Dead,
                    },
                    "alive" | "status:alive" => ClaimAssertion::LifeStatus {
                        status: LifeStatus::Alive,
                    },
                    _ => ClaimAssertion::Attribute {
                        operation: AttributeOperation::Add,
                        key: "status".into(),
                        value: ClaimScalar::Text {
                            value: status.clone(),
                        },
                        nature: AttributeNature::Unknown,
                    },
                };
                add(resolve(&entry.character_id, known), assertion, None);
            }
        }
        if !entry.changes.emotional_state.is_empty() {
            out.unsupported_fields
                .push(format!("character_states[{i}].emotional_state"));
        }
        if entry.changes.goals.is_some() {
            out.unsupported_fields
                .push(format!("character_states[{i}].goals"));
        }
        if entry.changes.notes.is_some() {
            out.unsupported_fields
                .push(format!("character_states[{i}].notes"));
        }
    }
    for (i, fact) in package.canonical_facts.iter().enumerate() {
        let key = fact.predicate.as_ref().or(fact.key.as_ref());
        let value = drafter_scalar(fact);
        if let (Some(key), Some(value)) = (key.filter(|k| !k.trim().is_empty()), value) {
            let subject = fact
                .subject_id
                .as_deref()
                .map(|id| resolve(id, known))
                .unwrap_or_else(|| ClaimEntity::Unresolved {
                    label: format!("legacy fact subject: {key}"),
                });
            add(
                subject,
                ClaimAssertion::Attribute {
                    operation: AttributeOperation::Set,
                    key: key.clone(),
                    value,
                    nature: match fact.scope {
                        Some(crate::models::CanonicalFactScope::Invariant) => {
                            AttributeNature::Invariant
                        }
                        Some(crate::models::CanonicalFactScope::Evolving) => {
                            AttributeNature::Evolving
                        }
                        Some(crate::models::CanonicalFactScope::Conditional) => {
                            AttributeNature::Conditional
                        }
                        None => AttributeNature::Unknown,
                    },
                },
                fact.valid_from.clone(),
            );
        } else {
            out.unsupported_fields
                .push(format!("canonical_facts[{i}].subject_predicate_value"));
        }
        if fact.scope.is_some() || fact.valid_until.is_some() || fact.supersedes_fact_id.is_some() {
            out.unsupported_fields
                .push(format!("canonical_facts[{i}].scope_validity_supersession"));
        }
    }
    for entry in &package.relationship_updates {
        add(
            resolve(&entry.character_a_id, known),
            ClaimAssertion::Relationship {
                operation: RelationshipOperation::Delta,
                other: resolve(&entry.character_b_id, known),
                trust: entry.trust_delta,
                tension: entry.tension_delta,
            },
            None,
        );
    }
    if !package.continuity_notes.is_empty() {
        out.unsupported_fields.push("continuity_notes".into());
    }
    out
}
pub(super) fn key_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
}
pub(super) fn normalized_decimal(text: &str) -> String {
    let negative = text.starts_with('-');
    let (whole, fraction) = text
        .trim_start_matches('-')
        .split_once('.')
        .unwrap_or((text.trim_start_matches('-'), ""));
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && (whole != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    format!(
        "{sign}{whole}{}{}",
        if fraction.is_empty() { "" } else { "." },
        fraction
    )
}
pub(super) fn normalized_assertion(assertion: &ClaimAssertion) -> ClaimAssertion {
    let mut value = assertion.clone();
    match &mut value {
        ClaimAssertion::Action { verb, .. } => *verb = key_text(verb),
        ClaimAssertion::Quantity {
            measure,
            amount,
            unit,
            ..
        } => {
            *measure = key_text(measure);
            *amount = normalized_decimal(amount);
            *unit = unit.trim().to_string();
        }
        ClaimAssertion::Attribute { key, value, .. } => {
            *key = key_text(key);
            match value {
                ClaimScalar::Text { .. } => {}
                ClaimScalar::Decimal { value, unit } => {
                    *value = normalized_decimal(value);
                    if let Some(unit) = unit {
                        *unit = unit.trim().to_string();
                    }
                }
                ClaimScalar::Boolean { .. } => {}
            }
        }
        _ => {}
    }
    value
}
fn semantic_key(claim: &ClaimV1) -> String {
    let slot = match &claim.proposed.assertion {
        ClaimAssertion::LifeStatus { .. } => "life_status".into(),
        ClaimAssertion::Action { verb, .. } => format!("action:{}", key_text(verb)),
        ClaimAssertion::LocatedAt { .. } | ClaimAssertion::Move { .. } => "location".into(),
        ClaimAssertion::SceneClock { .. } => "clock".into(),
        ClaimAssertion::Possession { item, custody, .. } => {
            format!("possession:{item:?}:{custody:?}")
        }
        ClaimAssertion::Injury { injury_id, .. } => format!("injury:{injury_id}"),
        ClaimAssertion::Quantity {
            measure,
            unit,
            transaction_id,
            ..
        } => format!(
            "quantity:{}:{}:{transaction_id:?}",
            key_text(measure),
            key_text(unit)
        ),
        ClaimAssertion::Knowledge { proposition, .. } => format!("knowledge:{proposition:?}"),
        ClaimAssertion::Attribute { key, .. } => format!("attribute:{}", key_text(key)),
        ClaimAssertion::Relationship { other, .. } => format!("relationship:{other:?}"),
        ClaimAssertion::Promise { thread, .. } => format!("promise:{thread:?}"),
        ClaimAssertion::Rule { rule, .. } => format!("rule:{rule:?}"),
    };
    // Unknown order/scope remains distinct; it is never guessed from package prose.
    #[derive(Serialize)]
    struct Key<'a> {
        project: &'a str,
        branch: &'a str,
        scene: &'a Option<String>,
        revision: &'a str,
        text_hash: &'a str,
        subject: &'a ClaimEntity,
        slot: String,
        position: (i32, i32, Option<i32>, Option<u32>),
        clock: &'a Option<crate::models::StoryClock>,
        modality: ClaimModality,
        scope: TemporalScope,
        speaker: &'a Option<ClaimEntity>,
        experiencer: &'a Option<ClaimEntity>,
    }
    serde_json::to_string(&Key {
        project: &claim.binding.project_id,
        branch: &claim.binding.branch_id,
        scene: &claim.binding.scene_id,
        revision: &claim.binding.revision_id,
        text_hash: &claim.binding.accepted_text_sha256,
        subject: &claim.proposed.subject,
        slot,
        position: (
            claim.binding.position.book_number,
            claim.binding.position.chapter_number,
            claim.binding.position.scene_order,
            claim.proposed.within_scene_order,
        ),
        clock: &claim.proposed.in_world_time,
        modality: claim.proposed.modality,
        scope: claim.proposed.temporal_scope,
        speaker: &claim.proposed.speaker,
        experiencer: &claim.proposed.experiencer,
    })
    .expect("validated typed claim key serializes")
}
fn comparison_values(claims: &[ClaimV1]) -> Vec<String> {
    let mut observations = BTreeSet::new();
    let mut values = Vec::new();
    for claim in claims {
        let assertion = normalized_assertion(&claim.proposed.assertion);
        let observation = matches!(
            assertion,
            ClaimAssertion::LifeStatus { .. }
                | ClaimAssertion::LocatedAt { .. }
                | ClaimAssertion::SceneClock { .. }
                | ClaimAssertion::Attribute { .. }
                | ClaimAssertion::Injury { .. }
                | ClaimAssertion::Quantity {
                    operation: QuantityOperation::Set,
                    ..
                }
                | ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Observation,
                    ..
                }
        );
        let value = serde_json::to_string(&assertion).expect("typed assertion serializes");
        // Repeated scalar observations have one value; occurrences and additive
        // deltas retain their multiplicity rather than hiding a missing effect.
        if !observation || observations.insert(value.clone()) {
            values.push(value);
        }
    }
    values.sort();
    values
}
/// Group competing assertions by their semantic stream. Evidence and source are
/// preserved; confidence and package agreement grant no authority.
pub fn compare_claim_sources(extractor: &[ClaimV1], drafter: &[ClaimV1]) -> Vec<ClaimComparisonV1> {
    let mut groups: BTreeMap<String, (Vec<ClaimV1>, Vec<ClaimV1>)> = BTreeMap::new();
    for (source, independent) in [(extractor, true), (drafter, false)] {
        for claim in source {
            let pair = groups.entry(semantic_key(claim)).or_default();
            if independent {
                pair.0.push(claim.clone());
            } else {
                pair.1.push(claim.clone());
            }
        }
    }
    groups.into_values().map(|(extractor, drafter)| {
        let kind = if extractor.is_empty() { ClaimComparisonKind::DrafterOnly }
            else if drafter.is_empty() { ClaimComparisonKind::ExtractorOnly }
            else if comparison_values(&extractor) == comparison_values(&drafter) { ClaimComparisonKind::Agreed }
            else { ClaimComparisonKind::Incompatible };
        let recommended_resolution = match kind {
            ClaimComparisonKind::Agreed => "Agreement does not override prior canon; validate before any decision.",
            ClaimComparisonKind::ExtractorOnly => "Drafter package omits this evidence-backed claim; review its evidence and coverage.",
            ClaimComparisonKind::DrafterOnly => "Package assertion lacks independent support; inspect accepted prose before deciding.",
            ClaimComparisonKind::Incompatible => "Review both sources and quoted evidence; neither confidence score establishes truth.",
        }.into();
        ClaimComparisonV1 { kind, extractor, drafter, recommended_resolution }
    }).collect()
}
