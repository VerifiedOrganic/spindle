use crate::helpers::fact;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn alias_out_of_window_fixture_is_clean() {
    let facts = vec![fact(
        "canonical_fact:cole_age",
        "age.past_life",
        "number",
        &["Cole's age"],
        None,
        Some(37.0),
        None,
    )];
    let prose = "Cole's age was never spoken aloud in the archive hall where everyone argued about duty, legacy, weather, and tactics for hours before anyone finally muttered that he was forty-three when he died.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert!(violations.is_empty());
}
