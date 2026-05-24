use crate::helpers::fact;
use serde_json::json;
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn date_drift_fixture_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:coronation_year",
        "event.coronation_year",
        "date",
        &["the coronation"],
        None,
        None,
        Some(json!({ "year": 1987 })),
    )];
    let prose = "The coronation happened in 1989, two years after the fire.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
    assert!(violations[0].message.contains("date drift"));
}
