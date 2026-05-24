use crate::helpers::fact;
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn generic_string_fact_detects_mismatch_without_contrast_terms() {
    let facts = vec![fact(
        "canonical_fact:nyra_title",
        "nyra.title",
        "string",
        &["Nyra"],
        Some("Warden"),
        None,
        None,
    )];
    let prose = "Nyra title was captain during the storm.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
    assert!(violations[0].observed.contains("captain"));
}

#[test]
fn generic_string_fact_ignores_unrelated_copula_phrase() {
    let facts = vec![fact(
        "canonical_fact:nyra_title",
        "nyra.title",
        "string",
        &["Nyra"],
        Some("Warden"),
        None,
        None,
    )];
    let prose = "Nyra was exhausted after the march.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert!(violations.is_empty());
}
