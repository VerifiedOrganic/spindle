use serde_json::{Value, json};
use spindle_core::canonical_facts::CanonicalFactForValidation;

pub fn fact(
    id: &str,
    predicate: &str,
    value_kind: &str,
    aliases: &[&str],
    value_text: Option<&str>,
    value_number: Option<f64>,
    value_json: Option<Value>,
) -> CanonicalFactForValidation {
    CanonicalFactForValidation {
        canonical_fact_id: id.to_string(),
        predicate: predicate.to_string(),
        value_kind: value_kind.to_string(),
        value_text: value_text.map(ToString::to_string),
        value_number,
        value_unit: None,
        value_json,
        aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        valid_from: None,
        valid_until: None,
        legacy_untyped: false,
    }
}

pub fn list_json(required: &[&str], forbidden: &[&str]) -> Value {
    json!({
        "required": required,
        "forbidden": forbidden
    })
}
