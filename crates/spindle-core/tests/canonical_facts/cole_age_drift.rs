use crate::helpers::fact;
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn cole_age_drift_fixture_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:cole_age",
        "age.past_life",
        "number",
        &["Cole's age", "past life"],
        None,
        Some(37.0),
        None,
    )];
    let prose = "Cole's age in his past life was forty-three when he died.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].predicate, "age.past_life");
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
}
