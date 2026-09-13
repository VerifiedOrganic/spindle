//! Integration proof for the Phase 1 fiction anti-slop scanner.
//!
//! Run with `--nocapture` to print the soft-vs-hard report used in the PR.

use spindle_core::style::antislop::{
    AntiSlopReport, DEFAULT_CATALOG_MARKDOWN, NON_PORTS, ScanInput, Severity, ShelfPack, scan,
};

fn print_report(label: &str, report: &AntiSlopReport) {
    println!("=== {label} ===");
    println!(
        "pack={}@{} skipped={} hard_count={} soft_count={} rewrite_max_passes={}",
        report.pack_id,
        report.pack_version,
        report.skipped,
        report.hard_count,
        report.soft_count,
        report.rewrite_max_passes
    );
    for hit in &report.hits {
        println!(
            "  - {} {:?} rewrite={:?} excerpt={:?} hint={}",
            hit.shelf_id, hit.severity, hit.rewrite, hit.excerpt, hit.rewrite_hint
        );
    }
}

#[test]
fn loads_phase0_pack_and_catalog() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    assert_eq!(pack.domain, "fiction");
    assert_eq!(pack.shelves.len(), 12);
    assert!(DEFAULT_CATALOG_MARKDOWN.contains("## This is not an AI detector"));
    for id in [
        "contrast_not_x_but_y",
        "emotion_cocktail",
        "fishing_ending",
        "said_bookism",
        "solitary_fade",
    ] {
        assert!(pack.shelf(id).is_some(), "missing shelf {id}");
    }
}

#[test]
fn solitary_fade_is_soft_not_in_hard_count() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let prose = "She spent the afternoon thinking about what he'd said.\n\n\
                 Hours passed.\n\n\
                 Later, at the meeting, she told him everything.";
    let report = scan(&pack, &ScanInput::fiction(prose));
    print_report("solitary_fade (must stay soft)", &report);

    let fades: Vec<_> = report
        .hits
        .iter()
        .filter(|hit| hit.shelf_id == "solitary_fade")
        .collect();
    assert!(!fades.is_empty(), "expected solitary_fade hits");
    assert!(fades.iter().all(|hit| hit.severity == Severity::Soft));
    assert_eq!(
        report.hard_count, 0,
        "solitary_fade must not increment hard_count"
    );
    assert!(report.soft_count >= 1);
}

#[test]
fn hard_shelves_count_only_when_over_limit() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let within = scan(
        &pack,
        &ScanInput::fiction("It wasn't anger. It was disappointment. He set the cup down."),
    );
    print_report("one contrast (within limit => hard_count 0)", &within);
    assert_eq!(within.hard_count, 0);

    let over = scan(
        &pack,
        &ScanInput::fiction(
            "It wasn't anger. It was disappointment.\n\
             This wasn't a homecoming. It was a reckoning.\n\
             She felt a mix of relief and dread.",
        ),
    );
    print_report("hard over-limit (contrast + cocktail)", &over);
    assert_eq!(over.hard_count, 2);
    assert!(
        over.hits
            .iter()
            .all(|hit| hit.shelf_id != "solitary_fade" || hit.severity == Severity::Soft)
    );
}

#[test]
fn said_bookism_over_limit_is_soft_only() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let report = scan(
        &pack,
        &ScanInput::fiction(
            "\"Leave,\" she hissed.\n\"Please,\" he breathed.\n\"Impossible,\" she gasped.\n",
        ),
    );
    print_report("said_bookism product lock (soft-only)", &report);
    assert!(
        report
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "said_bookism" && hit.severity == Severity::Soft)
    );
    assert_eq!(report.hard_count, 0);
}

#[test]
fn non_ports_are_absent() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    for id in NON_PORTS {
        assert!(pack.shelf(id).is_none());
    }
}
