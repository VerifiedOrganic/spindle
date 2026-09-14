//! Integration proof for the Phase 1 fiction anti-slop scanner.
//!
//! Run with `--nocapture` to print the soft-vs-hard report used in the PR.

use spindle_core::style::antislop::{
    ANTI_SLOP_CHECK, AntiSlopReport, DEFAULT_CATALOG_MARKDOWN, NON_PORTS, ScanInput, Severity,
    ShelfPack, auto_strict_blocks, compact_shelf_digest, dual_persona_injection,
    render_compact_shelf_digest_markdown, rewrite_from_beats_prompt, scan, verify_findings,
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

#[test]
fn compact_shelf_digest_is_the_writing_packet_slice() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let digest = compact_shelf_digest(&pack, None);
    print_report(
        "compact_shelf_digest (packet slice)",
        &scan(&pack, &ScanInput::fiction("She locked the door.")),
    );
    println!(
        "digest pack={}@{} shelves={} catalog={} rewrite_max_passes={}",
        digest.pack_id,
        digest.pack_version,
        digest.shelves.len(),
        digest.catalog_uri,
        digest.rewrite_max_passes
    );
    for shelf in &digest.shelves {
        println!(
            "  - {} {:?} enabled={} overlay={:?}",
            shelf.id, shelf.severity, shelf.enabled, shelf.profile_overlay
        );
    }
    assert_eq!(digest.shelves.len(), 12);
    assert!(
        digest
            .shelves
            .iter()
            .any(|shelf| shelf.id == "solitary_fade"
                && shelf.severity == Severity::Soft
                && shelf.enabled)
    );
    let markdown = render_compact_shelf_digest_markdown(&digest);
    println!("{markdown}");
    assert!(markdown.contains("compact_shelf_digest"));
    assert!(!markdown.contains("voice_samples"));
    assert!(markdown.contains("bible://references/anti-slop"));
}

#[test]
fn phase3_revise_loop_contracts() {
    let pack = ShelfPack::load_default().expect("v0 pack");
    let hard = scan(
        &pack,
        &ScanInput::fiction(
            "It wasn't anger. It was disappointment.\n\
             This wasn't a homecoming. It was a reckoning.\n\
             She felt a mix of relief and dread.",
        ),
    );
    print_report("phase3 hard verify (rewrite-from-beats)", &hard);
    let prompt = rewrite_from_beats_prompt(&hard);
    println!("{prompt}");
    assert!(prompt.contains("beat"));
    assert!(prompt.to_lowercase().contains("paraphrase"));
    let findings = verify_findings(&hard);
    assert!(findings.iter().all(|f| f.check_type == ANTI_SLOP_CHECK));
    assert!(auto_strict_blocks(&hard));
    let injection = dual_persona_injection(&hard);
    println!("{}", injection.craft_technician_block);
    println!("{}", injection.literary_critic_structure_block);
    assert!(injection.craft_technician_block.contains("NONE"));
    assert!(
        injection
            .literary_critic_structure_block
            .contains("structure")
    );

    let soft = scan(
        &pack,
        &ScanInput::fiction(
            "She spent the afternoon thinking about what he'd said.\n\nHours passed.",
        ),
    );
    print_report("phase3 soft-only (must not fail-close)", &soft);
    assert_eq!(soft.hard_count, 0);
    assert!(verify_findings(&soft).is_empty());
    assert!(!auto_strict_blocks(&soft));
}
