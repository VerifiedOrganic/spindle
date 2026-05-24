use crate::helpers::{fact, list_json};
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn list_required_missing_fixture_reports_soft_violation() {
    let facts = vec![fact(
        "canonical_fact:equipment",
        "equipment.ice_time",
        "list",
        &["private ice"],
        None,
        None,
        Some(list_json(&["skates", "stick"], &[])),
    )];
    let prose = "Before private ice, Cole grabbed his stick and sprinted for the door.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Soft);
    assert!(violations[0].observed.contains("skates"));
}
