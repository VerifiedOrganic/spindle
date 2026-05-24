use crate::helpers::fact;
use spindle_core::canonical_facts::{ViolationSeverity, validate_prose_against_facts};

#[test]
fn hair_color_contrast_fixture_reports_violation() {
    let facts = vec![fact(
        "canonical_fact:hair_color",
        "appearance.hair_color",
        "string",
        &["Mira's hair"],
        Some("brown"),
        None,
        None,
    )];
    let prose = "Mira's hair was blonde under the lantern light.";

    let violations = validate_prose_against_facts(prose, &facts);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].severity, ViolationSeverity::Hard);
    assert_eq!(violations[0].observed, "blonde");
}
