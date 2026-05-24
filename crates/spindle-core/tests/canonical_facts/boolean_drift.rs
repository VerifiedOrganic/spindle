use crate::helpers::fact;
use serde_json::json;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn boolean_mismatch_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:alive",
        "character.is_alive",
        "boolean",
        &["Mara"],
        None,
        None,
        Some(json!(true)),
    )];
    let prose = "Mara was dead before sunset.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].observed, "dead");
}
