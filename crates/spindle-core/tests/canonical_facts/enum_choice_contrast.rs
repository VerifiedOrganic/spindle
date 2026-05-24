use crate::helpers::fact;
use serde_json::json;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn enum_choice_mismatch_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:faction",
        "status.allegiance",
        "enum",
        &["allegiance"],
        Some("allies"),
        None,
        Some(json!({ "choices": ["allies", "neutral", "hostile"] })),
    )];
    let prose = "Their allegiance had shifted to hostile by dawn.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].observed, "hostile");
}
