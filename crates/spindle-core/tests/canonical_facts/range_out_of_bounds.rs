use crate::helpers::fact;
use serde_json::json;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn range_out_of_bounds_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:temperature",
        "weather.temperature",
        "range",
        &["temperature"],
        None,
        None,
        Some(json!({ "min": 10.0, "max": 20.0 })),
    )];
    let prose = "By noon the temperature reached 34 degrees in the square.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].observed, "34");
}
