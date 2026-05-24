use crate::helpers::fact;
use spindle_core::canonical_facts::validate_prose_against_facts;
use std::time::{Duration, Instant};

#[test]
fn performance_large_fact_set_fixture_stays_under_budget() {
    let mut facts = Vec::new();
    let mut prose_parts = Vec::new();

    for idx in 0..50 {
        let alias = format!("alias {}", idx);
        let predicate = format!("stats.value_{}", idx);
        let value = format!("value {}", idx);
        facts.push(fact(
            &format!("canonical_fact:{idx}"),
            &predicate,
            "string",
            &[&alias],
            Some(&value),
            None,
            None,
        ));
        prose_parts.push(format!(
            "In the long report, {alias} confirmed that {value} held steady at checkpoint {idx}."
        ));
    }

    while prose_parts.join(" ").split_whitespace().count() < 5000 {
        prose_parts.push("The watch rotated and the ledger remained unchanged.".to_string());
    }
    let prose = prose_parts.join(" ");

    let start = Instant::now();
    let violations = validate_prose_against_facts(&prose, &facts);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(100),
        "expected <100ms, got {elapsed:?}"
    );
    assert!(violations.is_empty());
}
