use crate::helpers::fact;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn cole_age_no_drift_fixture_is_clean() {
    let facts = vec![fact(
        "canonical_fact:cole_age",
        "age.past_life",
        "number",
        &["Cole's age"],
        None,
        Some(37.0),
        None,
    )];
    let prose = "Cole's age in his past life was thirty-seven when he died.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert!(violations.is_empty());
}
