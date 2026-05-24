use crate::helpers::{fact, list_json};
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn list_forbidden_item_outside_alias_window_is_ignored() {
    let facts = vec![fact(
        "canonical_fact:equipment",
        "equipment.ice_time",
        "list",
        &["private ice"],
        None,
        None,
        Some(list_json(&[], &["pads"])),
    )];
    let prose = "At private ice Cole skated laps and worked on stick handling while the rink lights buzzed overhead and the assistant coach timed every turn and shouted split times to the bench. Later, at home, he unpacked full pads by the door.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert!(violations.is_empty());
}

#[test]
fn list_required_item_outside_alias_window_does_not_satisfy_requirement() {
    let facts = vec![fact(
        "canonical_fact:equipment",
        "equipment.ice_time",
        "list",
        &["private ice"],
        None,
        None,
        Some(list_json(&["skates"], &[])),
    )];
    let prose = "At private ice Cole reviewed the game plan and stretched in silence. Hours later at home he cleaned his skates.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Soft);
    assert!(violations[0].observed.contains("skates"));
}
