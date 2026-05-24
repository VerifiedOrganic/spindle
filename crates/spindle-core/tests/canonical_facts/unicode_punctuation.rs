use crate::helpers::fact;
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn unicode_and_punctuation_fixture_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:cole_age",
        "age.past_life",
        "number",
        &["Cole’s age"],
        None,
        Some(37.0),
        None,
    )];
    let prose = "“Cole’s age”—forty-three, he said—never quite matched the ledger.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
}
