//! Phase 5: rolling chapter counters, learned suppressions, experimental flag.
//!
//! Product locks from Phases 0–4 must still hold: soft-on-save is not tested
//! here; `auto_strict` still requires `hard_count == 0`; `solitary_fade` stays
//! soft; experimental structural notes are never shelves or fail gates.

use spindle_core::style::antislop::{
    ChapterCounters, NON_PORTS, ScanInput, ScanOptions, Severity, ShelfPack, SuppressionStore,
    auto_strict_blocks, experimental_structural_default, learn_suppressions_from_edit, scan,
    scan_with_options, self_hosted_sampler_status, verify_findings,
};

fn pack() -> ShelfPack {
    ShelfPack::load_default().expect("v0 pack")
}

fn fiction(prose: &str) -> ScanInput<'_> {
    ScanInput::fiction(prose)
}

#[test]
fn chapter_quota_rolls_contrast_across_scenes() {
    let pack = pack();
    let scene1 = "It wasn't anger. It was disappointment. He set the cup down.";
    let scene2 = "This wasn't a homecoming. It was a reckoning. She locked the door.";

    let first = scan(&pack, &fiction(scene1));
    assert_eq!(
        first.hard_count, 0,
        "one contrast in scene 1 is within the chapter quota: {first:?}"
    );

    let mut counters = ChapterCounters::new();
    counters.add_scene(&pack, scene1);
    assert_eq!(counters.used("contrast_not_x_but_y"), 1);

    let second = scan_with_options(
        &pack,
        &fiction(scene2),
        ScanOptions {
            chapter_prior: Some(&counters),
            ..ScanOptions::default()
        },
    );
    assert!(
        second
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "contrast_not_x_but_y" && hit.severity == Severity::Hard),
        "scene 2 must consume the remaining chapter quota: {second:?}"
    );
    assert!(
        second.hard_count >= 1,
        "rolled contrast is a hard finding: {second:?}"
    );
    assert!(auto_strict_blocks(&second));
}

#[test]
fn said_bookism_rolls_across_chapter_but_stays_soft() {
    let pack = pack();
    let scene1 = "\"Leave,\" she hissed.\n\"Please,\" he breathed.\n";
    let scene2 = "\"Impossible,\" she gasped.\n";

    let first = scan(&pack, &fiction(scene1));
    assert_eq!(first.hard_count, 0);
    assert!(
        first.hits.iter().all(|hit| hit.shelf_id != "said_bookism"),
        "two tags are within the chapter quota of 2: {first:?}"
    );

    let mut counters = ChapterCounters::new();
    counters.add_scene(&pack, scene1);
    let second = scan_with_options(
        &pack,
        &fiction(scene2),
        ScanOptions {
            chapter_prior: Some(&counters),
            ..ScanOptions::default()
        },
    );
    assert!(
        second
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "said_bookism" && hit.severity == Severity::Soft),
        "third chapter tag is over-limit but still soft: {second:?}"
    );
    assert_eq!(second.hard_count, 0);
    assert!(!auto_strict_blocks(&second));
}

#[test]
fn scene_scoped_solitary_fade_does_not_roll() {
    let pack = pack();
    let fade = "She spent the afternoon thinking about what he'd said.\n\nHours passed.";
    let mut counters = ChapterCounters::new();
    counters.add_scene(&pack, fade);
    assert_eq!(
        counters.used("solitary_fade"),
        0,
        "scene-scoped shelves must not enter chapter counters"
    );

    let second = scan_with_options(
        &pack,
        &fiction("She stacked the mill ledgers and waited for the kettle."),
        ScanOptions {
            chapter_prior: Some(&counters),
            ..ScanOptions::default()
        },
    );
    assert!(
        second
            .hits
            .iter()
            .all(|hit| hit.shelf_id != "solitary_fade"),
        "scene-2 prose with no fade markers must not inherit scene-1 fade: {second:?}"
    );
}

#[test]
fn learn_and_apply_suppressions_from_kept_excerpt() {
    let pack = pack();
    let agent = "It wasn't anger. It was disappointment.\n\
                 This wasn't a homecoming. It was a reckoning.\n\
                 She felt a mix of relief and dread when the letter came.";
    let operator = "He set the cup down without drinking.\n\
                    She felt a mix of relief and dread when the letter came.";

    let learned = learn_suppressions_from_edit(&pack, agent, operator);
    assert!(
        learned.iter().any(|s| s.shelf_id == "emotion_cocktail"),
        "kept cocktail excerpt must become a suppression: {learned:?}"
    );

    let store = SuppressionStore::from_entries(learned);
    let unfiltered = scan(&pack, &fiction(agent));
    assert!(unfiltered.hard_count >= 2, "{unfiltered:?}");

    let filtered = scan_with_options(
        &pack,
        &fiction(agent),
        ScanOptions {
            suppressions: Some(&store),
            ..ScanOptions::default()
        },
    );
    assert!(
        filtered
            .hits
            .iter()
            .all(|hit| hit.shelf_id != "emotion_cocktail"),
        "suppression must drop the kept cocktail: {filtered:?}"
    );
    assert!(
        filtered.hard_count < unfiltered.hard_count,
        "applying suppressions must recount hard_count"
    );
    assert!(
        filtered
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "contrast_not_x_but_y"),
        "unrelated hard shelves stay: {filtered:?}"
    );
}

#[test]
fn suppression_learning_ignores_distant_typo_fix_and_scopes_narrowly() {
    let pack = pack();
    let agent = "The lantern hung by the stable door with teh iron latch.\n\n\
                 Far down the hall, the clerk waited by the grate.\n\n\
                 She felt a mix of relief and dread when the letter came.";
    // Operator only fixes a typo in paragraph 1; paragraph 3 was not in an edited region.
    let typo_fix = "The lantern hung by the stable door with the iron latch.\n\n\
                    Far down the hall, the clerk waited by the grate.\n\n\
                    She felt a mix of relief and dread when the letter came.";
    let learned_from_typo = learn_suppressions_from_edit(&pack, agent, typo_fix);
    assert!(
        learned_from_typo.is_empty(),
        "a typo fix in another paragraph must not suppress untouched hits: {learned_from_typo:?}"
    );

    // Same-paragraph minor typo fix must also not suppress.
    let single_para_agent = "He set teh cup down without drinking. \
                             She felt a mix of relief and dread when the letter came.";
    let single_para_typo = "He set the cup down without drinking. \
                            She felt a mix of relief and dread when the letter came.";
    assert!(
        learn_suppressions_from_edit(&pack, single_para_agent, single_para_typo).is_empty(),
        "a 1-word typo fix in the same paragraph must not suppress adjacent hits"
    );

    // When a marker sentence is deleted, the marker appearing elsewhere must not suppress.
    let marker_agent = "He shrugged and ran a hand through his hair. \"I don't know.\"\n\n\
                        She stacked the ledgers on the counter.";
    let marker_edit = "He wiped the cleaver on his apron. \"Ask her.\"\n\n\
                       She stacked the ledgers on the counter and shrugged at the rain.";
    assert!(
        learn_suppressions_from_edit(&pack, marker_agent, marker_edit).is_empty(),
        "deleted hit whose marker word appears elsewhere must not learn a suppression"
    );

    // A learned suppression from a genuinely edited region is scoped to its surrounding span,
    // not a blanket pass for the same phrase or marker in a different sentence/scene.
    let edited_agent = "It wasn't anger. It was disappointment.\n\
                        This wasn't a homecoming. It was a reckoning.\n\
                        She felt a mix of relief and dread when the letter came.";
    let edited_operator = "He set the cup down without drinking.\n\
                           She felt a mix of relief and dread when the letter came.";
    let learned = learn_suppressions_from_edit(&pack, edited_agent, edited_operator);
    assert_eq!(learned.len(), 1);
    let store = SuppressionStore::from_entries(learned);

    let other_scene = "At the harbor gate, Marcus felt a mix of relief and dread as the bell rang.";
    let other_report = scan_with_options(
        &pack,
        &fiction(other_scene),
        ScanOptions {
            suppressions: Some(&store),
            ..ScanOptions::default()
        },
    );
    assert!(
        other_report
            .hits
            .iter()
            .any(|hit| hit.shelf_id == "emotion_cocktail"),
        "learned suppression must not suppress a different sentence in another scene: {other_report:?}"
    );
}

#[test]
fn experimental_structural_is_off_by_default_and_not_a_gate() {
    assert!(!experimental_structural_default());

    let pack = pack();
    let lecture = "She locked the till.\n\n\
                   And that was the point. The moral of the story was that \
                   an old novel she had loved could not save anyone.";

    let off = scan(&pack, &fiction(lecture));
    assert!(
        off.structural_observations.is_empty(),
        "default scan must not emit experimental observations: {off:?}"
    );
    assert_eq!(off.hard_count, 0);
    assert!(verify_findings(&off).is_empty());
    assert!(!auto_strict_blocks(&off));

    let on = scan_with_options(
        &pack,
        &fiction(lecture),
        ScanOptions {
            experimental_structural: true,
            ..ScanOptions::default()
        },
    );
    assert!(
        !on.structural_observations.is_empty(),
        "flag-on must emit StoryScope-inspired observations: {on:?}"
    );
    assert_eq!(
        on.hard_count, off.hard_count,
        "observations must not increment hard_count"
    );
    assert!(verify_findings(&on).is_empty());
    assert!(!auto_strict_blocks(&on));
    for obs in &on.structural_observations {
        assert!(
            !NON_PORTS.contains(&obs.question_id.as_str()),
            "observations must not be Voices/tech non-ports"
        );
        assert_ne!(obs.question_id, "solitary_fade");
        assert_ne!(obs.question_id, "bluf");
    }
}

#[test]
fn self_hosted_sampler_is_out_of_band() {
    let status = self_hosted_sampler_status();
    assert_eq!(status.as_str(), "out_of_band");
    assert!(
        !status.in_process(),
        "do not ship a half-baked in-process sampler"
    );
}
