use crate::helpers::{fact, list_json};
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn list_forbidden_present_fixture_reports_hard_violation() {
    let facts = vec![fact(
        "canonical_fact:equipment",
        "equipment.ice_time",
        "list",
        &["private ice"],
        None,
        None,
        Some(list_json(&["skates", "stick"], &["pads"])),
    )];
    let prose = "At private ice he wore skates, carried his stick, and hauled full pads.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
    assert_eq!(violations[0].observed, "pads");
}
