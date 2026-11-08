use super::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtractionValidationError {
    pub code: ExtractionValidationCode,
    pub claim_index: Option<usize>,
    /// A schema path only; never scene text, evidence, or model output.
    pub field: String,
}
impl std::fmt::Display for ExtractionValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} at {} (claim {:?})",
            self.code, self.field, self.claim_index
        )
    }
}
impl std::error::Error for ExtractionValidationError {}
fn error(code: ExtractionValidationCode, field: &str) -> ExtractionValidationError {
    ExtractionValidationError {
        code,
        claim_index: None,
        field: field.into(),
    }
}
pub fn exact_text_sha256(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn validate_binding(
    binding: &ClaimBindingV1,
    text: &str,
) -> Result<(), ExtractionValidationError> {
    if binding.schema_version != CLAIM_SCHEMA_VERSION {
        return Err(error(ExtractionValidationCode::Schema, "schema_version"));
    }
    if [
        &binding.project_id,
        &binding.branch_id,
        &binding.revision_id,
        &binding.extraction_run_id,
        &binding.model,
        &binding.prompt_version,
    ]
    .iter()
    .any(|v| v.trim().is_empty())
        || binding
            .scene_id
            .as_ref()
            .is_some_and(|s| s.trim().is_empty())
        || binding.position.book_number < 1
        || binding.position.chapter_number < 1
        || binding.position.scene_order.is_none_or(|s| s < 1)
    {
        return Err(error(ExtractionValidationCode::Identity, "binding"));
    }
    if binding.accepted_text_sha256 != exact_text_sha256(text) {
        return Err(error(
            ExtractionValidationCode::Hash,
            "accepted_text_sha256",
        ));
    }
    Ok(())
}
fn entity(value: &ClaimEntity, known: &BTreeSet<String>) -> Result<(), ExtractionValidationError> {
    let valid = match value {
        ClaimEntity::Resolved { id } => {
            let shaped = id.split_once(':').is_some_and(|(kind, key)| {
                !kind.is_empty()
                    && kind.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && !key.is_empty()
                    && key
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            });
            shaped && known.contains(id)
        }
        ClaimEntity::Unresolved { label } => !label.trim().is_empty(),
    };
    if valid {
        Ok(())
    } else {
        Err(error(ExtractionValidationCode::Entity, "entity_reference"))
    }
}
fn nonempty(value: &str) -> Result<(), ExtractionValidationError> {
    if value.trim().is_empty() {
        Err(error(ExtractionValidationCode::Value, "assertion"))
    } else {
        Ok(())
    }
}
/// Decimal syntax is checked without a lossy binary floating-point conversion.
pub(super) fn decimal(value: &str) -> Result<(), ExtractionValidationError> {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || parts.next().is_some()
        || fraction.is_some_and(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(error(ExtractionValidationCode::Value, "decimal"));
    }
    Ok(())
}
fn clock(value: &StoryClock) -> Result<(), ExtractionValidationError> {
    let field = if value
        .duration_days
        .is_some_and(|d| !d.is_finite() || d < 0.0)
    {
        Some("clock.duration_days")
    } else if value.time_of_day.is_some_and(|m| m < 0) {
        Some("clock.time_of_day")
    } else if value
        .precision
        .as_ref()
        .is_some_and(|p| p.trim().is_empty())
    {
        Some("clock.precision")
    } else {
        None
    };
    field.map_or(Ok(()), |field| {
        Err(error(ExtractionValidationCode::Value, field))
    })
}

fn assertion(
    value: &ClaimAssertion,
    known: &BTreeSet<String>,
) -> Result<(), ExtractionValidationError> {
    match value {
        ClaimAssertion::LifeStatus { .. } => Ok(()),
        ClaimAssertion::Action { verb, .. } => nonempty(verb),
        ClaimAssertion::LocatedAt { location } => entity(location, known),
        ClaimAssertion::Move { from, to } => {
            entity(from, known)?;
            entity(to, known)
        }
        ClaimAssertion::SceneClock { clock: value } => clock(value),
        ClaimAssertion::Possession {
            operation,
            item,
            counterparty,
            ..
        } => {
            entity(item, known)?;
            if *operation == PossessionOperation::Transfer && counterparty.is_none() {
                return Err(error(
                    ExtractionValidationCode::Entity,
                    "possession.counterparty",
                ));
            }
            if let Some(other) = counterparty {
                entity(other, known)?;
            }
            Ok(())
        }
        ClaimAssertion::Injury {
            injury_id,
            body_site,
            ..
        } => {
            nonempty(injury_id)?;
            nonempty(body_site)
        }
        ClaimAssertion::Quantity {
            measure,
            amount,
            unit,
            transaction_id,
            ..
        } => {
            nonempty(measure)?;
            decimal(amount)?;
            nonempty(unit)?;
            if let Some(id) = transaction_id {
                nonempty(id)?;
            }
            Ok(())
        }
        ClaimAssertion::Knowledge {
            proposition,
            audience,
            source,
            operation,
        } => {
            entity(proposition, known)?;
            if *operation == KnowledgeOperation::Disclose && audience.is_empty() {
                return Err(error(
                    ExtractionValidationCode::Entity,
                    "knowledge.audience",
                ));
            }
            for other in audience.iter().chain(source.iter()) {
                entity(other, known)?;
            }
            Ok(())
        }
        ClaimAssertion::Attribute { key, value, .. } => {
            nonempty(key)?;
            match value {
                ClaimScalar::Text { value } => nonempty(value),
                ClaimScalar::Boolean { .. } => Ok(()),
                ClaimScalar::Decimal { value, unit } => {
                    decimal(value)?;
                    if let Some(unit) = unit {
                        nonempty(unit)?;
                    }
                    Ok(())
                }
            }
        }
        ClaimAssertion::Relationship { other, .. } => entity(other, known),
        ClaimAssertion::Promise { thread, .. } => entity(thread, known),
        ClaimAssertion::Rule {
            rule,
            anchor,
            preconditions,
            effects,
            ..
        } => {
            entity(rule, known)?;
            if let Some(anchor) = anchor {
                entity(anchor, known)?;
            }
            for text in preconditions.iter().chain(effects) {
                nonempty(text)?;
            }
            Ok(())
        }
    }
}
/// Shared provider/host acceptance gate. It never persists or ratifies claims.
pub fn bind_extraction(
    payload: ExtractionPayloadV1,
    binding: ClaimBindingV1,
    text: &str,
    known: &BTreeSet<String>,
) -> Result<ExtractionReportV1, ExtractionValidationError> {
    validate_binding(&binding, text)?;
    if serde_json::to_vec(&payload).map_or(true, |bytes| bytes.len() > MAX_EXTRACTION_PAYLOAD_BYTES)
    {
        return Err(error(ExtractionValidationCode::Limit, "payload_bytes"));
    }
    if payload.schema_version != CLAIM_SCHEMA_VERSION {
        return Err(error(ExtractionValidationCode::Schema, "schema_version"));
    }
    if payload.accepted_text_sha256 != binding.accepted_text_sha256 {
        return Err(error(
            ExtractionValidationCode::Hash,
            "accepted_text_sha256",
        ));
    }
    if payload.summary.trim().is_empty() {
        return Err(error(ExtractionValidationCode::Value, "summary"));
    }
    if payload.claims.len() > 128 {
        return Err(error(ExtractionValidationCode::Limit, "claims"));
    }
    if payload.coverage.len() != CLAIM_FAMILIES.len()
        || CLAIM_FAMILIES
            .iter()
            .any(|f| !payload.coverage.contains_key(f))
    {
        return Err(error(ExtractionValidationCode::Coverage, "coverage"));
    }
    let mut ids = BTreeSet::new();
    for (index, claim) in payload.claims.iter().enumerate() {
        let checked = (|| {
            if claim.claim_id.trim().is_empty() || !ids.insert(&claim.claim_id) {
                return Err(error(ExtractionValidationCode::DuplicateClaim, "claim_id"));
            }
            entity(&claim.subject, known)?;
            for other in claim.speaker.iter().chain(claim.experiencer.iter()) {
                entity(other, known)?;
            }
            assertion(&claim.assertion, known)?;
            if let Some(time) = &claim.in_world_time {
                clock(time)?;
            }
            if claim
                .confidence
                .is_none_or(|c| !c.is_finite() || !(0.0..=1.0).contains(&c))
            {
                return Err(error(ExtractionValidationCode::Confidence, "confidence"));
            }
            if claim.evidence.is_empty() || claim.evidence.len() > 16 {
                return Err(error(ExtractionValidationCode::Evidence, "evidence"));
            }
            for evidence in &claim.evidence {
                if evidence.start_byte >= evidence.end_byte
                    || text.get(evidence.start_byte..evidence.end_byte)
                        != Some(evidence.quote.as_str())
                {
                    return Err(error(ExtractionValidationCode::Evidence, "evidence.span"));
                }
            }
            Ok(())
        })();
        checked.map_err(|mut error: ExtractionValidationError| {
            error.claim_index = Some(index);
            error
        })?;
    }
    for (family, coverage) in &payload.coverage {
        let present = payload
            .claims
            .iter()
            .any(|c| c.assertion.family() == *family);
        let valid = match coverage {
            FamilyCoverage::InspectedNone => !present,
            FamilyCoverage::ClaimsPresent => present,
            FamilyCoverage::Unknown { reason } => !reason.trim().is_empty(),
        };
        if !valid {
            return Err(error(ExtractionValidationCode::Coverage, "coverage.status"));
        }
    }
    Ok(ExtractionReportV1 {
        claims: payload
            .claims
            .into_iter()
            .map(|proposed| ClaimV1 {
                binding: binding.clone(),
                source: ClaimSource::Independent,
                proposed,
            })
            .collect(),
        binding,
        summary: payload.summary,
        tone: payload.tone,
        coverage: payload.coverage,
    })
}
