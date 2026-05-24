use crate::helpers::fact;
use spindle_core::canonical_facts::validate_prose_against_facts;

#[test]
fn legacy_untyped_facts_are_skipped() {
    let mut stale = fact(
        "canonical_fact:legacy",
        "age.past_life",
        "number",
        &["Cole's age"],
        None,
        Some(37.0),
        None,
    );
    stale.legacy_untyped = true;
    let prose = "Cole's age in his past life was forty-three.";

    let violations = validate_prose_against_facts(prose, &[stale]);
    assert!(violations.is_empty());
}
